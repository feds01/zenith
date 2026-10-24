//! The tests of `zenith lsp`: they start the binary as an editor would, on a
//! project in a temporary directory, and read the diagnostics it publishes.

use std::{
    collections::BTreeMap,
    env, fs,
    io::{BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread,
    time::{Duration, Instant},
};

use lsp_server::{Message, Notification, Request, RequestId};
use lsp_types::{
    Diagnostic, DiagnosticRelatedInformation, DiagnosticSeverity, DiagnosticTag,
    DidSaveTextDocumentParams, InitializeParams, Location, MessageType, NumberOrString, Position,
    PublishDiagnosticsParams, Range, ShowMessageParams, TextDocumentIdentifier, Url,
    WorkspaceFolder,
    notification::{
        DidSaveTextDocument, Exit, Initialized, Notification as _, PublishDiagnostics, ShowMessage,
    },
    request::{Initialize, Request as _, Shutdown},
};
use serde_json::json;
use tempfile::TempDir;

/// How long to wait for a message from the server before failing.
const TIMEOUT: Duration = Duration::from_secs(60);

/// A file with characters of two and four bytes, which are one and two
/// UTF-16 code units, before its import on the second line.
const IMPORTS_DIRECTLY: &str = "// The columns of the next line are counted in UTF-16 code units.\n\
     /* café 😀 */ import { b } from \"../api/b\";\n";

/// A file with a suppression that has no effect, as no import is on its
/// line.
const UNUSED_SUPPRESSION: &str = "export const e = 1; // zenith-ignore web-api\n";

/// `zenith lsp`, started in the directory above a project, as an editor that
/// opened the project's folder.
struct Editor {
    /// The server's process.
    server: Child,

    /// What the editor writes to the server.
    stdin: ChildStdin,

    /// The messages that the server writes, as they are read.
    messages: Receiver<Message>,
}

impl Editor {
    /// Start the server in the parent of `project`, and initialise it with
    /// `project` as the workspace folder, without saying which encodings of
    /// positions the editor supports.
    fn start(project: &Path) -> Self {
        let mut server = Command::new(env!("CARGO_BIN_EXE_zenith"))
            .args(["lsp", "--no-cache"])
            .current_dir(project.parent().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("zenith lsp starts");
        let stdin = server.stdin.take().unwrap();
        let mut stdout = BufReader::new(server.stdout.take().unwrap());
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            while let Ok(Some(message)) = Message::read(&mut stdout) {
                if sender.send(message).is_err() {
                    break;
                }
            }
        });
        let mut editor = Self { server, stdin, messages };

        let uri = Url::from_directory_path(project).unwrap();
        let params = InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder { uri, name: "project".into() }]),
            ..InitializeParams::default()
        };
        editor.send(Request::new(RequestId::from(1), Initialize::METHOD.into(), params).into());
        let Message::Response(response) = editor.receive() else {
            panic!("expected the response to `initialize`");
        };
        let capabilities = &response.response_result.unwrap()["capabilities"];
        assert_eq!(capabilities["positionEncoding"], "utf-16");
        editor.send(Notification::new(Initialized::METHOD.into(), json!({})).into());
        editor
    }

    /// Send `message` to the server.
    fn send(&mut self, message: Message) {
        message.write(&mut self.stdin).unwrap();
        self.stdin.flush().unwrap();
    }

    /// The next message from the server.
    fn receive(&self) -> Message {
        self.messages.recv_timeout(TIMEOUT).expect("a message from zenith lsp")
    }

    /// The diagnostics of the next `count` files that the server publishes,
    /// by the path of each file relative to `project`.
    fn published(&self, project: &Path, count: usize) -> BTreeMap<String, Vec<Diagnostic>> {
        (0..count)
            .map(|_| {
                let Message::Notification(notification) = self.receive() else {
                    panic!("expected diagnostics");
                };
                assert_eq!(notification.method, PublishDiagnostics::METHOD);
                let params: PublishDiagnosticsParams =
                    serde_json::from_value(notification.params).unwrap();
                (relative(project, &params.uri), params.diagnostics)
            })
            .collect()
    }

    /// The kind of the next message that the server shows, and the message.
    fn shown(&self) -> (MessageType, String) {
        let Message::Notification(notification) = self.receive() else {
            panic!("expected a message to show");
        };
        assert_eq!(notification.method, ShowMessage::METHOD);
        let params: ShowMessageParams = serde_json::from_value(notification.params).unwrap();
        (params.typ, params.message)
    }

    /// Tell the server that the file at `path` has been saved.
    fn save(&mut self, path: &Path) {
        let uri = Url::from_file_path(path).unwrap();
        let params =
            DidSaveTextDocumentParams { text_document: TextDocumentIdentifier { uri }, text: None };
        self.send(Notification::new(DidSaveTextDocument::METHOD.into(), params).into());
    }

    /// Shut the server down, and wait for it to exit.
    fn shut_down(mut self) {
        self.send(Request::new(RequestId::from(2), Shutdown::METHOD.into(), json!(null)).into());
        let Message::Response(response) = self.receive() else {
            panic!("expected the response to `shutdown`");
        };
        assert_eq!(response.id, RequestId::from(2));
        self.send(Notification::new(Exit::METHOD.into(), json!(null)).into());
        let start = Instant::now();
        let status = loop {
            if let Some(status) = self.server.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > TIMEOUT {
                let _ = self.server.kill();
                panic!("zenith lsp did not exit");
            }
            thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success());
    }
}

/// A project in a temporary directory, by its canonical path, as zenith
/// names it, with `web` importing `api` directly and through `lib`.
fn project() -> (TempDir, PathBuf) {
    let dir = TempDir::new().unwrap();
    // `/tmp` is a symlink on macOS. Windows paths lose their `\\?\` prefix, as
    // zenith's do.
    let root = fs::canonicalize(dir.path()).unwrap();
    let root = PathBuf::from(root.to_string_lossy().trim_start_matches(r"\\?\"));
    for (path, contents) in [
        ("zenith.toml", include_str!("fixtures/lsp.toml")),
        ("web/a.ts", IMPORTS_DIRECTLY),
        ("web/d.ts", "import { c } from \"../lib/c\";\n"),
        ("lib/c.ts", "export { b as c } from \"../api/b\";\n"),
        ("web/e.ts", UNUSED_SUPPRESSION),
        ("api/b.ts", "export const b = 1;\n"),
    ] {
        fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
        fs::write(root.join(path), contents).unwrap();
    }
    (dir, root)
}

/// The path of the file `uri` relative to `project`, with `/` separators.
fn relative(project: &Path, uri: &Url) -> String {
    let path = uri.to_file_path().unwrap();
    path.strip_prefix(project).unwrap().to_string_lossy().replace('\\', "/")
}

/// A range on one line.
fn range(line: u32, start: u32, end: u32) -> Range {
    Range::new(Position::new(line, start), Position::new(line, end))
}

/// A diagnostic of the contract `web-api`, with its guidance.
fn diagnostic(
    range: Range,
    title: &str,
    related: Option<Vec<DiagnosticRelatedInformation>>,
) -> Diagnostic {
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String("web-api".into())),
        source: Some("zenith".into()),
        message: format!("{title}\nMove what `web` needs out of `api`."),
        related_information: related,
        ..Diagnostic::default()
    }
}

#[test]
fn publishes_violations_on_start_and_on_save() {
    let (_dir, project) = project();
    let mut editor = Editor::start(&project);

    // 1. On start, each file that imports `api` has a diagnostic on its import, and
    //    the one that imports it through `lib` has the rest of the chain as related
    //    information. Each says how to fix it.
    let via_lib = "`web` must not import `api` (via web/d.ts -> lib/c.ts -> api/b.ts); remove \
                   this import, or one further along the chain";
    let lib_import = DiagnosticRelatedInformation {
        location: Location {
            uri: Url::from_file_path(project.join("lib/c.ts")).unwrap(),
            range: range(0, 23, 33),
        },
        message: "imports `api/b.ts`".into(),
    };
    // `/* café 😀 */ import { b } from ` is 31 characters, 32 UTF-16 code units
    // and 35 bytes.
    let direct =
        diagnostic(range(1, 32, 42), "`web` must not import `api`; remove this import", None);
    let through_lib = diagnostic(range(0, 18, 28), via_lib, Some(vec![lib_import]));
    // The suppression that has no effect is a hint, which editors fade.
    let unused = Diagnostic {
        range: range(0, 20, 44),
        severity: Some(DiagnosticSeverity::HINT),
        source: Some("zenith".into()),
        message: "unused suppression: no import on line 1 for the contracts it names".into(),
        tags: Some(vec![DiagnosticTag::UNNECESSARY]),
        ..Diagnostic::default()
    };
    assert_eq!(
        editor.published(&project, 3),
        BTreeMap::from([
            ("web/a.ts".to_string(), vec![direct]),
            ("web/d.ts".to_string(), vec![through_lib]),
            ("web/e.ts".to_string(), vec![unused]),
        ])
    );

    // 2. Once the direct import is gone and the file saved, its diagnostic is
    //    cleared. The others stay, and are not sent again.
    fs::write(project.join("web/a.ts"), "export const a = 1;\n").unwrap();
    editor.save(&project.join("web/a.ts"));
    assert_eq!(editor.published(&project, 1), BTreeMap::from([("web/a.ts".to_string(), vec![])]));

    // 3. A mistake in the configuration is shown where it is, and the other files
    //    keep their diagnostics.
    fs::write(project.join("zenith.toml"), include_str!("fixtures/lsp-invalid.toml")).unwrap();
    editor.save(&project.join("zenith.toml"));
    let invalid = Diagnostic {
        range: range(6, 0, 7),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("zenith".into()),
        message: "unknown key `severty`\ndid you mean `severity`?".into(),
        ..Diagnostic::default()
    };
    assert_eq!(
        editor.published(&project, 1),
        BTreeMap::from([("zenith.toml".to_string(), vec![invalid])])
    );

    // 4. Once it is fixed, it is cleared. The check finds what it did before, so
    //    nothing else is sent: the server answers the next request straight after.
    fs::write(project.join("zenith.toml"), include_str!("fixtures/lsp.toml")).unwrap();
    editor.save(&project.join("zenith.toml"));
    assert_eq!(
        editor.published(&project, 1),
        BTreeMap::from([("zenith.toml".to_string(), vec![])])
    );

    editor.shut_down();
}

#[test]
fn counts_the_findings_that_have_no_place_in_a_file() {
    let (_dir, project) = project();
    fs::write(project.join("zenith.toml"), include_str!("fixtures/lsp-limits.toml")).unwrap();
    let editor = Editor::start(&project);

    // The suppression, which names a contract that no longer exists, has a place
    // in its file.
    let published = editor.published(&project, 1);
    assert_eq!(published.keys().collect::<Vec<_>>(), ["web/e.ts"]);
    // A limit that `api` exceeds is about no one import, so it is counted in a
    // message instead.
    let message = "1 finding has no place in a file to show it; run `zenith check` to see it";
    assert_eq!(editor.shown(), (MessageType::WARNING, message.to_string()));

    editor.shut_down();
}

#[cfg(unix)]
#[test]
fn names_files_as_the_editor_names_its_folder() {
    // The editor opens the project through a symlink, which zenith resolves.
    let (_dir, project) = project();
    let links = TempDir::new().unwrap();
    let opened = links.path().join("project");
    std::os::unix::fs::symlink(&project, &opened).unwrap();
    let editor = Editor::start(&opened);

    // The files are named under the folder as the editor opened it, related
    // information included.
    let published = editor.published(&opened, 3);
    let files: Vec<_> = published.keys().map(String::as_str).collect();
    assert_eq!(files, ["web/a.ts", "web/d.ts", "web/e.ts"]);
    let related = published["web/d.ts"][0].related_information.as_ref().unwrap();
    assert_eq!(relative(&opened, &related[0].location.uri), "lib/c.ts");

    editor.shut_down();
}
