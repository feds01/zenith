//! A commit of a project's repository as a [`FileSystem`], read straight from
//! git's object store, for reading the project as it was at that commit
//! without checking it out.

use std::{
    fmt,
    io::{self, BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, ChildStdout, Stdio},
    sync::{Mutex, PoisonError},
};

use rustc_hash::{FxHashMap, FxHashSet};
use zenith_diagnostics::message;
use zenith_utils::{
    fs::{DirEntry, FileKind, FileSystem, OsFileSystem},
    path::{absolute_path, normalize_path, relative_posix},
};

use super::{Git, GitError, fields};

/// The most links that a path may lead through, about as many as operating
/// systems allow.
const MAX_LINKS: usize = 40;

impl Git {
    /// The commit that `reference` names, e.g. `main` or `HEAD~2`, as a file
    /// system. The reference is resolved as by [`Self::resolve_commit`], with
    /// its errors.
    ///
    /// Nothing is written: neither the repository's index nor its worktrees
    /// change. Nor does git fetch what it can be kept from fetching: the files
    /// that Git LFS stores are read as their pointers, and a partial clone is
    /// asked not to fetch the objects that it is missing, which recent
    /// versions of git heed.
    pub fn commit_files(&self, reference: &str) -> Result<CommitFileSystem, GitError> {
        // 1. Find the commit, so that a reference that names none says so.
        let commit = self.resolve_commit(reference)?;

        // 2. List its tree: each file, link and directory, by its path in the worktree,
        //    with the directories that hold them. A submodule is an empty directory.
        let listing = self.run(&["ls-tree", "-r", "-t", "-z", "--full-tree", &commit])?;
        let mut entries = FxHashMap::default();
        entries.insert(String::new(), Entry { kind: FileKind::Directory, id: String::new() });
        for line in fields(&listing) {
            let Some((meta, path)) = line.split_once('\t') else {
                continue;
            };
            let mut meta = meta.split(' ');
            let (Some(mode), Some(_), Some(id)) = (meta.next(), meta.next(), meta.next()) else {
                continue;
            };
            let kind = match mode {
                "040000" | "160000" => FileKind::Directory,
                "120000" => FileKind::Symlink,
                _ => FileKind::File,
            };
            entries.insert(path.to_owned(), Entry { kind, id: id.to_owned() });
        }
        let mut children: FxHashMap<String, Vec<String>> = FxHashMap::default();
        for path in entries.keys().filter(|path| !path.is_empty()) {
            let (dir, name) = path.rsplit_once('/').unwrap_or(("", path));
            children.entry(dir.to_owned()).or_default().push(name.to_owned());
        }

        // 3. Find what git ignores in the worktree, which is read from the disk.
        let ignored = self.run(&[
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
        ])?;
        let ignored = fields(&ignored).map(|path| path.trim_end_matches('/').to_owned()).collect();

        // 4. Start the process that reads the objects.
        let objects = Mutex::new(Objects::start(self)?);
        let worktree = self.worktree.clone();
        Ok(CommitFileSystem { commit, worktree, entries, children, ignored, objects })
    }
}

/// A commit of a project's repository, as a file system, made by
/// [`Git::commit_files`].
///
/// It holds the commit's files at their paths in the worktree, and reads
/// them from git's object store, following the links among them. The paths
/// that git ignores in the worktree, such as installed packages, and those
/// outside the worktree, are read from the disk, so that what the project
/// imports from them resolves as it does in the worktree. Every other path is
/// missing: the worktree's untracked files are not part of the commit.
pub struct CommitFileSystem {
    /// The id of the commit.
    commit: String,

    /// The root of the worktree, which is canonical.
    worktree: PathBuf,

    /// Each file, link and directory of the commit, by its POSIX path
    /// relative to the worktree's root, which is `""`.
    entries: FxHashMap<String, Entry>,

    /// The names in each directory of the commit, by its path.
    children: FxHashMap<String, Vec<String>>,

    /// The paths that git ignores in the worktree, relative to its root: the
    /// files, and the directories, each of which ignores what is in it.
    ignored: FxHashSet<String>,

    /// The process that reads the objects, which one thread at a time uses.
    objects: Mutex<Objects>,
}

/// A file, link or directory of a commit.
#[derive(Debug)]
struct Entry {
    /// What it is.
    kind: FileKind,

    /// The id of its object, whose contents are a link's target.
    id: String,
}

/// Where a path leads, in the commit or on the disk.
enum Location<'a> {
    /// The entry of the commit at this POSIX path relative to the worktree's
    /// root.
    Commit(String, &'a Entry),

    /// This path on the disk.
    Disk(PathBuf),
}

impl CommitFileSystem {
    /// The id of the commit.
    pub fn commit(&self) -> &str {
        &self.commit
    }

    /// Where `path` leads, through the links on the way to it, and through
    /// the one it ends with if `follow` is set.
    fn locate(&self, path: &Path, follow: bool) -> io::Result<Location<'_>> {
        self.locate_from(normalize_path(path), follow, 0)
    }

    /// [`Self::locate`], having followed `links` links to the normalised
    /// `path`.
    fn locate_from(&self, path: PathBuf, follow: bool, links: usize) -> io::Result<Location<'_>> {
        // 1. Outside the worktree, the path is on the disk.
        let Some(relative) = relative_posix(&self.worktree, &path) else {
            return Ok(Location::Disk(path));
        };
        if relative.is_empty() {
            return Ok(Location::Commit(relative, &self.entries[""]));
        }

        // 2. Look each prefix of the path up in the commit, from the root down.
        let names: Vec<&str> = relative.split('/').collect();
        let mut current = String::new();
        for (index, name) in names.iter().enumerate() {
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(name);
            let last = index + 1 == names.len();

            // 3. A path that the commit doesn't have is on the disk if git ignores it, and
            //    missing otherwise.
            let Some(entry) = self.entries.get(&current) else {
                return match self.ignores(&current) {
                    true => Ok(Location::Disk(path)),
                    false => Err(io::ErrorKind::NotFound.into()),
                };
            };

            // 4. A link leads on from its target, which is relative to the directory that
            //    holds it.
            match entry.kind {
                FileKind::Symlink if follow || !last => {
                    if links == MAX_LINKS {
                        return Err(io::Error::other("too many levels of symbolic links"));
                    }
                    let target = self.target(entry)?;
                    let link = absolute_path(&self.worktree, &current);
                    let mut next = link.parent().unwrap_or(&self.worktree).join(target);
                    next.extend(&names[index + 1..]);
                    return self.locate_from(normalize_path(next), follow, links + 1);
                }
                _ if last => return Ok(Location::Commit(current, entry)),
                FileKind::Directory => {}
                _ => return Err(io::ErrorKind::NotADirectory.into()),
            }
        }
        Err(io::ErrorKind::NotFound.into())
    }

    /// Whether git ignores the path `relative` to the worktree's root, or a
    /// directory that holds it.
    fn ignores(&self, relative: &str) -> bool {
        let mut dirs = relative.match_indices('/').map(|(end, _)| &relative[..end]);
        self.ignored.contains(relative) || dirs.any(|dir| self.ignored.contains(dir))
    }

    /// The contents of the object of `entry`.
    fn contents(&self, entry: &Entry) -> io::Result<Vec<u8>> {
        self.objects.lock().unwrap_or_else(PoisonError::into_inner).read(&entry.id)
    }

    /// The target of the link `entry`.
    fn target(&self, entry: &Entry) -> io::Result<PathBuf> {
        Ok(PathBuf::from(String::from_utf8_lossy(&self.contents(entry)?).into_owned()))
    }
}

impl FileSystem for CommitFileSystem {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        match self.locate(path, true)? {
            Location::Commit(_, entry) if entry.kind == FileKind::File => self.contents(entry),
            Location::Commit(..) => Err(io::ErrorKind::IsADirectory.into()),
            Location::Disk(path) => OsFileSystem.read(&path),
        }
    }

    fn kind(&self, path: &Path) -> io::Result<FileKind> {
        match self.locate(path, true)? {
            Location::Commit(_, entry) => Ok(entry.kind),
            Location::Disk(path) => OsFileSystem.kind(&path),
        }
    }

    fn link_kind(&self, path: &Path) -> io::Result<FileKind> {
        match self.locate(path, false)? {
            Location::Commit(_, entry) => Ok(entry.kind),
            Location::Disk(path) => OsFileSystem.link_kind(&path),
        }
    }

    /// The entries of the directory in the commit, and those on the disk that
    /// git ignores, such as an installed `node_modules`.
    fn read_dir(&self, path: &Path) -> io::Result<Vec<DirEntry>> {
        let relative = match self.locate(path, true)? {
            Location::Commit(relative, entry) if entry.kind == FileKind::Directory => relative,
            Location::Commit(..) => return Err(io::ErrorKind::NotADirectory.into()),
            Location::Disk(path) => return OsFileSystem.read_dir(&path),
        };
        let join = |name: &str| match relative.is_empty() {
            true => name.to_owned(),
            false => format!("{relative}/{name}"),
        };
        let names = self.children.get(&relative).into_iter().flatten();
        let mut entries: Vec<_> = names
            .map(|name| DirEntry { path: path.join(name), kind: self.entries[&join(name)].kind })
            .collect();
        let on_disk = OsFileSystem.read_dir(&absolute_path(&self.worktree, &relative));
        for entry in on_disk.into_iter().flatten() {
            let Some(name) = entry.path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let child = join(name);
            if !self.entries.contains_key(&child) && self.ignores(&child) {
                entries.push(DirEntry { path: path.join(name), kind: entry.kind });
            }
        }
        Ok(entries)
    }

    fn read_link(&self, path: &Path) -> io::Result<PathBuf> {
        match self.locate(path, false)? {
            Location::Commit(_, entry) if entry.kind == FileKind::Symlink => self.target(entry),
            Location::Commit(..) => Err(io::ErrorKind::InvalidInput.into()),
            Location::Disk(path) => OsFileSystem.read_link(&path),
        }
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        match self.locate(path, true)? {
            Location::Commit(relative, _) => Ok(absolute_path(&self.worktree, &relative)),
            Location::Disk(path) => OsFileSystem.canonicalize(&path),
        }
    }

    /// The files and links of the commit under `dir`, which are all of the
    /// commit's, without the worktree's ignored files.
    fn listed_files(&self, dir: &Path) -> Option<Vec<DirEntry>> {
        let Ok(Location::Commit(relative, _)) = self.locate(dir, true) else {
            return Some(Vec::new());
        };
        let prefix = match relative.is_empty() {
            true => String::new(),
            false => format!("{relative}/"),
        };
        let files = self.entries.iter().filter_map(|(path, entry)| {
            let inside = path.strip_prefix(&prefix).filter(|inside| !inside.is_empty())?;
            (entry.kind != FileKind::Directory)
                .then(|| DirEntry { path: absolute_path(dir, inside), kind: entry.kind })
        });
        Some(files.collect())
    }
}

impl fmt::Debug for CommitFileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommitFileSystem")
            .field("commit", &self.commit)
            .field("worktree", &self.worktree)
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

/// A `git cat-file --batch` process, which reads objects by their ids.
struct Objects {
    /// The process.
    process: Child,

    /// Where the ids are written.
    input: ChildStdin,

    /// Where the objects are read from.
    output: BufReader<ChildStdout>,

    /// Whether a read failed partway, after which what the process prints is
    /// out of step with what is asked of it, so nothing more is read.
    stopped: bool,
}

impl Objects {
    /// Start the process in the worktree of `git`, without letting a partial
    /// clone fetch the objects that it is missing.
    fn start(git: &Git) -> Result<Self, GitError> {
        let mut process = git
            .command(&["cat-file", "--batch"])
            .env("GIT_NO_LAZY_FETCH", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| GitError::Run(error.to_string()))?;
        let (Some(input), Some(output)) = (process.stdin.take(), process.stdout.take()) else {
            return Err(GitError::Run("`git cat-file` has no input or output".into()));
        };
        Ok(Self { process, input, output: BufReader::new(output), stopped: false })
    }

    /// The contents of the blob `id`. A blob that git doesn't have is
    /// [`io::ErrorKind::NotFound`]; any other failure stops the reads.
    fn read(&mut self, id: &str) -> io::Result<Vec<u8>> {
        if self.stopped {
            return Err(io::Error::other(message!(workspace_objects_stopped)));
        }
        let read = self.ask(id);
        if read.as_ref().is_err_and(|error| error.kind() != io::ErrorKind::NotFound) {
            self.stopped = true;
        }
        read
    }

    /// Ask the process for the blob `id`, and read its contents.
    fn ask(&mut self, id: &str) -> io::Result<Vec<u8>> {
        // 1. Ask for it, and read what git says of it: `<id> blob <size>`, or `<id>
        //    missing`. Nothing at all means that the process has stopped.
        writeln!(self.input, "{id}")?;
        self.input.flush()?;
        let mut header = String::new();
        if self.output.read_line(&mut header)? == 0 {
            return Err(io::Error::other(message!(workspace_objects_stopped)));
        }
        let header = header.trim_end();
        let fields: Vec<_> = header.split(' ').collect();
        let size = match fields[..] {
            [_, "missing"] => {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    message!(workspace_object_missing, id),
                ));
            }
            [_, "blob", size] => size.parse().ok(),
            _ => None,
        };
        let Some(size) = size else {
            let error = message!(workspace_object_unexpected, id, header);
            return Err(io::Error::new(io::ErrorKind::InvalidData, error));
        };

        // 2. Read its contents, then the newline that ends them.
        let mut contents = vec![0; size];
        self.output.read_exact(&mut contents)?;
        self.output.read_exact(&mut [0])?;
        Ok(contents)
    }
}

impl Drop for Objects {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use zenith_utils::fs::adjust_canonicalisation;

    use super::*;
    use crate::{git::tests::committed, testing::commit};

    /// Stop `process`, and the processes that it started. On Windows, `git`
    /// is Git for Windows' launcher, which runs git as a process of its own:
    /// killing the launcher alone leaves git running, and answering.
    fn stop(process: &mut Child) {
        #[cfg(windows)]
        {
            let pid = process.id().to_string();
            let taskkill = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid])
                .status()
                .unwrap();
            assert!(taskkill.success());
        }
        #[cfg(not(windows))]
        process.kill().unwrap();
        process.wait().unwrap();
    }

    /// The paths and kinds of `entries`, sorted.
    fn sorted(entries: Vec<DirEntry>) -> Vec<(PathBuf, FileKind)> {
        let mut entries: Vec<_> =
            entries.into_iter().map(|entry| (entry.path, entry.kind)).collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    #[test]
    fn reads_the_files_of_the_commit_rather_than_the_worktree() {
        let (_dir, root) = committed(&["web/src/a.ts", "web/b.ts"]);
        fs::write(root.join("web/src/a.ts"), "changed").unwrap();
        fs::remove_file(root.join("web/b.ts")).unwrap();
        fs::write(root.join("web/c.ts"), "").unwrap();

        let git = Git::new(&root);
        let files = git.commit_files("HEAD").unwrap();
        assert_eq!(files.commit(), git.resolve_commit("HEAD").unwrap());
        let a = files.read_text(&root.join("web/src/a.ts")).unwrap();
        assert_eq!(a, "export const name = 'web/src/a.ts';\n");
        assert!(files.is_file(&root.join("web/b.ts")));
        assert!(files.is_dir(&root.join("web/src")));
        let missing = files.kind(&root.join("web/c.ts")).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound);
        let canonical = files.canonicalize(&root.join("web/src/../b.ts")).unwrap();
        assert_eq!(canonical, root.join("web/b.ts"));
        assert!(!files.on_disk());

        let web = files.read_dir(&root.join("web")).unwrap();
        let expected =
            [(root.join("web/b.ts"), FileKind::File), (root.join("web/src"), FileKind::Directory)];
        assert_eq!(sorted(web), expected);
        let listed = files.listed_files(&root.join("web")).unwrap();
        let expected =
            [(root.join("web/b.ts"), FileKind::File), (root.join("web/src/a.ts"), FileKind::File)];
        assert_eq!(sorted(listed), expected);
    }

    #[test]
    fn what_git_ignores_and_what_is_outside_the_worktree_are_on_the_disk() {
        let (_dir, root) = committed(&["web/a.ts"]);
        fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
        commit(&root);
        fs::create_dir_all(root.join("node_modules/react")).unwrap();
        fs::write(root.join("node_modules/react/package.json"), "{}").unwrap();
        fs::write(root.join("scratch.ts"), "").unwrap();
        let elsewhere = TempDir::new().unwrap();
        let outside = adjust_canonicalisation(elsewhere.path()).join("a.json");
        fs::write(&outside, "{}").unwrap();

        let files = Git::new(&root).commit_files("HEAD").unwrap();
        let installed = files.read_text(&root.join("node_modules/react/package.json"));
        assert_eq!(installed.unwrap(), "{}");
        assert_eq!(files.read_text(&outside).unwrap(), "{}");
        assert!(!files.is_file(&root.join("scratch.ts")));

        let at_root = files.read_dir(&root).unwrap();
        let expected = [
            (root.join(".gitignore"), FileKind::File),
            (root.join("node_modules"), FileKind::Directory),
            (root.join("web"), FileKind::Directory),
        ];
        assert_eq!(sorted(at_root), expected);
        let listed = sorted(files.listed_files(&root).unwrap());
        assert_eq!(
            listed,
            [(root.join(".gitignore"), FileKind::File), (root.join("web/a.ts"), FileKind::File)]
        );
    }

    #[cfg(unix)]
    #[test]
    fn follows_the_links_of_the_commit() {
        let (_dir, root) = committed(&["src/a.ts"]);
        std::os::unix::fs::symlink("src", root.join("link")).unwrap();
        commit(&root);
        // The link is gone from the worktree, but not from the commit.
        fs::remove_file(root.join("link")).unwrap();

        let files = Git::new(&root).commit_files("HEAD").unwrap();
        let link = root.join("link");
        assert_eq!(files.kind(&link).unwrap(), FileKind::Directory);
        assert_eq!(files.link_kind(&link).unwrap(), FileKind::Symlink);
        assert_eq!(files.read_link(&link).unwrap(), Path::new("src"));
        let a = files.read_text(&link.join("a.ts")).unwrap();
        assert_eq!(a, "export const name = 'src/a.ts';\n");
        assert_eq!(files.canonicalize(&link.join("a.ts")).unwrap(), root.join("src/a.ts"));
        let listed = sorted(files.listed_files(&root).unwrap());
        assert_eq!(listed, [(link, FileKind::Symlink), (root.join("src/a.ts"), FileKind::File)]);
    }

    #[test]
    fn a_missing_object_is_not_found_but_a_stopped_reader_fails_every_read() {
        let (_dir, root) = committed(&["a.ts"]);
        let files = Git::new(&root).commit_files("HEAD").unwrap();
        let missing = Entry { kind: FileKind::File, id: "0".repeat(40) };
        assert_eq!(files.contents(&missing).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert!(files.is_file(&root.join("a.ts")) && files.read(&root.join("a.ts")).is_ok());

        // Without the process, no file can be read, and none is taken for missing.
        stop(&mut files.objects.lock().unwrap().process);
        for _ in 0..2 {
            let error = files.read(&root.join("a.ts")).unwrap_err();
            assert_ne!(error.kind(), io::ErrorKind::NotFound, "{error}");
        }
    }

    #[test]
    fn a_reference_that_names_no_commit_is_unknown() {
        let (_dir, root) = committed(&["a.ts"]);
        // `-p` is not taken for an option of git's.
        for reference in ["no-such-branch", "-p", "HEAD~5"] {
            let error = Git::new(&root).commit_files(reference).unwrap_err();
            assert_eq!(error, GitError::UnknownCommit(reference.to_owned()), "{reference}");
        }

        let dir = TempDir::new().unwrap();
        let root = adjust_canonicalisation(dir.path());
        // A repository above the temporary directory would know its commits.
        if !root.ancestors().any(|dir| dir.join(".git").exists()) {
            let error = Git::new(&root).commit_files("HEAD").unwrap_err();
            assert_eq!(error, GitError::NotARepository(root));
        }
    }
}
