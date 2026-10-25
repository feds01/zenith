//! The server: how it starts with the editor, and the loop that checks the
//! project and publishes what each check finds.

use std::{
    any::Any,
    collections::{BTreeMap, BTreeSet},
    mem,
    panic::{self, AssertUnwindSafe},
    path::{Path, PathBuf},
};

use log::info;
use lsp_server::{
    Connection, ErrorCode, IoThreads, Message, Notification, Request, RequestId, Response,
};
use lsp_types::{
    Diagnostic, DidChangeWatchedFilesParams, DidChangeWatchedFilesRegistrationOptions,
    DidSaveTextDocumentParams, FileSystemWatcher, GlobPattern, InitializeParams, InitializeResult,
    MessageType, PublishDiagnosticsParams, Registration, RegistrationParams, ServerCapabilities,
    ServerInfo, ShowMessageParams, TextDocumentSyncCapability, TextDocumentSyncOptions,
    TextDocumentSyncSaveOptions, Url,
    notification::{
        DidChangeWatchedFiles, DidSaveTextDocument, Exit, Notification as _, PublishDiagnostics,
        ShowMessage,
    },
    request::{RegisterCapability, Request as _},
};
use zenith_diagnostics::{Report, Reporter, SourceMap, message};

use crate::{diagnostics::Editor, error::ServerError};

/// What the server's operations return, failing with why it stopped talking to
/// the editor.
type Result<T, E = ServerError> = std::result::Result<T, E>;

/// The id of the request that asks the editor to report the changes to files,
/// and of the registration it asks for.
const WATCH_FILES: &str = "zenith/watch-files";

/// What may have changed in the project since the last check, so that a
/// check can redo only what the changes affect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changes {
    /// Anything may have: no check has run yet, or the editor doesn't report
    /// the changes to files.
    Unknown,

    /// The files at these paths, and nothing else, as the editor reported
    /// them: those it saved, and those that it saw change on the disk, such
    /// as when a branch is checked out. The paths of the files in the folder
    /// that it opened are as zenith names them.
    Files(BTreeSet<PathBuf>),
}

/// What a check found, for the server to publish.
#[derive(Debug, Default)]
pub struct Found {
    /// The sources that the reports point into.
    pub sources: SourceMap,

    /// The reports of what is wrong, such as the contracts that the code
    /// breaks.
    pub reports: Vec<Report>,

    /// The reports of code that has no effect, such as suppressions, which
    /// editors fade.
    pub unnecessary: Vec<Report>,
}

/// Why a check could not run.
#[derive(Debug)]
pub enum Failure {
    /// What stops it, such as errors in the configuration file, as reports
    /// that point into the sources.
    Reports(SourceMap, Vec<Report>),

    /// What stops it, as a message.
    Message(String),
}

/// A language server that publishes what checking the project finds, as
/// diagnostics, when it starts and whenever the editor saves a file.
pub struct Server {
    /// The channels to and from the editor.
    connection: Connection,

    /// The threads that read and write stdin and stdout, for a server on
    /// them.
    io_threads: Option<IoThreads>,

    /// How the editor shows diagnostics.
    editor: Editor,

    /// The diagnostics of each file that has any, as the last check that ran
    /// found them.
    found: BTreeMap<Url, Vec<Diagnostic>>,

    /// The diagnostics of why the latest check failed, if it did, by file.
    failed: BTreeMap<Url, Vec<Diagnostic>>,

    /// The diagnostics that the editor shows, of each file that has any.
    shown: BTreeMap<Url, Vec<Diagnostic>>,

    /// The titles of the reports of the last check that ran that have no
    /// diagnostic, as the editor was last told of them.
    unlocated: Vec<String>,

    /// Whether the editor reports the changes to files, once it was asked to.
    watching: bool,

    /// What changed since the last check, as the editor reported it.
    changed: Changes,
}

impl Server {
    /// A server on stdin and stdout, once the editor has initialised it.
    pub fn stdio() -> Result<Self> {
        let (connection, io_threads) = Connection::stdio();
        Ok(Self { io_threads: Some(io_threads), ..Self::new(connection)? })
    }

    /// A server on `connection`, once the editor has initialised it: it
    /// tells the editor that it wants to hear of saves, agrees an encoding of
    /// positions with it, and asks it to report the changes to files, if it
    /// can.
    pub fn new(connection: Connection) -> Result<Self> {
        let (id, params) = connection.initialize_start()?;
        let params: InitializeParams = serde_json::from_value(params)?;
        let editor = Editor::new(&params.capabilities, opened(&params));
        let sync = TextDocumentSyncOptions {
            save: Some(TextDocumentSyncSaveOptions::Supported(true)),
            ..TextDocumentSyncOptions::default()
        };
        let result = InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(editor.position_encoding()),
                text_document_sync: Some(TextDocumentSyncCapability::Options(sync)),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "zenith".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        };
        connection.initialize_finish(id, serde_json::to_value(result)?)?;
        let watching = editor.watches_files;
        let server = Self {
            connection,
            io_threads: None,
            editor,
            found: BTreeMap::new(),
            failed: BTreeMap::new(),
            shown: BTreeMap::new(),
            unlocated: Vec::new(),
            watching,
            changed: Changes::unchanged(watching),
        };
        if watching {
            server.watch_files()?;
        }
        Ok(server)
    }

    /// The folder that the editor opened, as it names it, if it said.
    pub fn folder(&self) -> Option<&Path> {
        self.editor.folder.as_ref().map(|folder| folder.path.as_path())
    }

    /// Publish what `check` finds now, and again whenever the editor saves a
    /// file, until it shuts the server down or exits. Each check is told what
    /// changed since the last. A check that fails, or panics, keeps the
    /// diagnostics of the last one that ran, and shows why it failed. Returns
    /// whether the editor shut the server down before it exited, as the
    /// protocol asks it to.
    pub fn run(mut self, mut check: impl FnMut(Changes) -> Result<Found, Failure>) -> Result<bool> {
        let mut check = |changes| {
            panic::catch_unwind(AssertUnwindSafe(|| check(changes))).unwrap_or_else(|panic| {
                let reason = panic_reason(&*panic);
                Err(Failure::Message(message!(lsp_check_panicked, reason)))
            })
        };

        // 1. Publish what the project has to begin with.
        self.publish(check(Changes::Unknown))?;

        // 2. Handle what the editor sends, until it shuts the server down, exits or
        //    goes away.
        let mut shut_down = false;
        while let Ok(message) = self.connection.receiver.recv() {
            match message {
                Message::Request(request) => {
                    if self.connection.handle_shutdown(&request)? {
                        shut_down = true;
                        break;
                    }
                    self.on_request(request)?;
                }
                Message::Notification(notification) if notification.method == Exit::METHOD => {
                    break;
                }
                Message::Notification(notification) => {
                    self.on_notification(notification, &mut check)?;
                }
                Message::Response(response) => self.on_response(response),
            }
        }

        // 3. Let the last messages be written, once nothing more will be sent.
        let Self { connection, io_threads, .. } = self;
        drop(connection);
        if let Some(io_threads) = io_threads {
            io_threads.join()?;
        }
        Ok(shut_down)
    }

    /// Answer a request from the editor. The server answers none but
    /// `shutdown` yet, so it refuses each, as the protocol asks of a method
    /// that it doesn't know.
    fn on_request(&self, request: Request) -> Result<()> {
        let error = message!(lsp_unknown_request, method = request.method);
        let code = ErrorCode::MethodNotFound as i32;
        self.send(Response::new_err(request.id, code, error).into())
    }

    /// Act on a notification from the editor: check the project again when
    /// it saves a file, and keep the changes to files that it reports for the
    /// next check. Others are ignored, as the protocol allows.
    fn on_notification(
        &mut self,
        notification: Notification,
        check: &mut impl FnMut(Changes) -> Result<Found, Failure>,
    ) -> Result<()> {
        match notification.method.as_str() {
            DidSaveTextDocument::METHOD => {
                let saved = serde_json::from_value(notification.params);
                let saved =
                    saved.ok().map(|params: DidSaveTextDocumentParams| params.text_document);
                let next = Changes::unchanged(self.watching);
                let changes = match (mem::replace(&mut self.changed, next), saved) {
                    (Changes::Files(mut files), Some(saved)) => {
                        files.extend(self.editor.path(&saved.uri));
                        Changes::Files(files)
                    }
                    // Without the file that was saved, anything may have changed.
                    _ => Changes::Unknown,
                };
                self.publish(check(changes))
            }
            DidChangeWatchedFiles::METHOD => {
                let watched = serde_json::from_value(notification.params);
                match (&mut self.changed, watched) {
                    (Changes::Files(files), Ok(DidChangeWatchedFilesParams { changes })) => {
                        let editor = &self.editor;
                        files.extend(changes.iter().filter_map(|change| editor.path(&change.uri)));
                    }
                    (changed, _) => *changed = Changes::Unknown,
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Act on the editor's answer to a request: if it won't report the
    /// changes to files, anything may change from then on.
    fn on_response(&mut self, response: Response) {
        if response.id == RequestId::from(WATCH_FILES.to_string())
            && let Err(error) = response.response_result
        {
            info!("{}", message!(lsp_not_watching, error = error.message));
            (self.watching, self.changed) = (false, Changes::Unknown);
        }
    }

    /// Ask the editor to report the changes to every file, which it may
    /// refuse in its answer.
    fn watch_files(&self) -> Result<()> {
        let glob_pattern = GlobPattern::String("**/*".to_string());
        let watchers = vec![FileSystemWatcher { glob_pattern, kind: None }];
        let options = DidChangeWatchedFilesRegistrationOptions { watchers };
        let registration = Registration {
            id: WATCH_FILES.to_string(),
            method: DidChangeWatchedFiles::METHOD.to_string(),
            register_options: Some(serde_json::to_value(options)?),
        };
        let params = RegistrationParams { registrations: vec![registration] };
        let id = RequestId::from(WATCH_FILES.to_string());
        self.send(Request::new(id, RegisterCapability::METHOD.to_string(), params).into())
    }

    /// Publish what a check found, or why it failed. A check that fails
    /// keeps the diagnostics of the last one that ran, and adds those of why
    /// it failed, until a check runs again. Only the files whose diagnostics
    /// change are sent, a file that no longer has any with none.
    fn publish(&mut self, checked: Result<Found, Failure>) -> Result<()> {
        // 1. Keep what the check found, or why it failed, and what to tell the editor
        //    of it.
        let message = match checked {
            Ok(found) => self.keep_found(found),
            Err(failure) => self.keep_failure(failure),
        };

        // 2. Send the diagnostics of each file whose diagnostics change: those of the
        //    last check that ran, and then those of why the latest failed.
        let mut shown = self.found.clone();
        for (uri, failed) in &self.failed {
            shown.entry(uri.clone()).or_default().extend(failed.iter().cloned());
        }
        let files: BTreeSet<_> = self.shown.keys().chain(shown.keys()).collect();
        let changed = files.into_iter().filter(|uri| self.shown.get(*uri) != shown.get(*uri));
        let sent = changed.map(|uri| (uri.clone(), shown.get(uri).cloned().unwrap_or_default()));
        self.send_diagnostics(sent.collect())?;
        self.shown = shown;

        // 3. Tell the editor what has no place in a file.
        match message {
            Some((typ, message)) => self.show(typ, message),
            None => Ok(()),
        }
    }

    /// Keep the diagnostics of what a check found, and return the message
    /// that counts the reports that have none, unless the editor was last
    /// told of the same ones.
    fn keep_found(&mut self, found: Found) -> Option<(MessageType, String)> {
        let Found { sources, reports, unnecessary } = found;
        let diagnostics = self.editor.diagnostics(&sources, &reports, &unnecessary);
        let count: usize = diagnostics.files.values().map(Vec::len).sum();
        let files = diagnostics.files.len();
        info!("{}", message!(lsp_published, diagnostics = count, files));

        (self.found, self.failed) = (diagnostics.files, BTreeMap::new());
        let titles: Vec<_> =
            diagnostics.unlocated.iter().map(|report| report.title.clone()).collect();
        let changed = titles != self.unlocated && !titles.is_empty();
        let message =
            changed.then(|| (MessageType::WARNING, message!(lsp_unlocated, count = titles.len())));
        self.unlocated = titles;
        message
    }

    /// Keep the diagnostics of why a check failed, and return the message
    /// that shows what has none: the reports that have no diagnostic, or the
    /// failure's message.
    fn keep_failure(&mut self, failure: Failure) -> Option<(MessageType, String)> {
        let (failed, message) = match failure {
            Failure::Reports(sources, reports) => {
                let diagnostics = self.editor.diagnostics(&sources, &reports, &[]);
                let unlocated: Vec<_> = diagnostics.unlocated.into_iter().cloned().collect();
                let message = (!unlocated.is_empty())
                    .then(|| Reporter::new(&sources, &unlocated).short().to_string());
                (diagnostics.files, message)
            }
            Failure::Message(message) => (BTreeMap::new(), Some(message)),
        };
        self.failed = failed;
        message.map(|message| (MessageType::ERROR, message))
    }

    /// Send the diagnostics of each file, replacing those it had.
    fn send_diagnostics(&self, files: BTreeMap<Url, Vec<Diagnostic>>) -> Result<()> {
        for (uri, diagnostics) in files {
            let params = PublishDiagnosticsParams { uri, diagnostics, version: None };
            let notification = Notification::new(PublishDiagnostics::METHOD.to_string(), params);
            self.send(notification.into())?;
        }
        Ok(())
    }

    /// Show the editor a message, of the kind `typ`.
    fn show(&self, typ: MessageType, message: String) -> Result<()> {
        let params = ShowMessageParams { typ, message };
        let notification = Notification::new(ShowMessage::METHOD.to_string(), params);
        self.send(notification.into())
    }

    /// Send `message` to the editor, unless it went away.
    fn send(&self, message: Message) -> Result<()> {
        self.connection.sender.send(message).map_err(|_| ServerError::Disconnected)
    }
}

impl Changes {
    /// No change yet, since a check, as an editor that reports changes to
    /// files if `watching` would see it.
    fn unchanged(watching: bool) -> Self {
        match watching {
            true => Self::Files(BTreeSet::new()),
            false => Self::Unknown,
        }
    }
}

/// The folder that the editor opened, if it said: the first of its
/// workspace folders, or else its root.
fn opened(params: &InitializeParams) -> Option<PathBuf> {
    let mut folders = params.workspace_folders.iter().flatten();
    let folder = folders.find_map(|folder| folder.uri.to_file_path().ok());
    // Editors that predate workspace folders name their root instead.
    #[allow(deprecated)]
    let root = || {
        let uri = params.root_uri.as_ref().and_then(|uri| uri.to_file_path().ok());
        uri.or_else(|| params.root_path.as_ref().map(PathBuf::from))
    };
    folder.or_else(root)
}

/// What a panic says, if it says it as text.
fn panic_reason(panic: &(dyn Any + Send)) -> String {
    match (panic.downcast_ref::<&str>(), panic.downcast_ref::<String>()) {
        (Some(reason), _) => reason.to_string(),
        (_, Some(reason)) => reason.clone(),
        (None, None) => message!(lsp_unknown_panic),
    }
}

#[cfg(test)]
mod tests {
    use std::{env, sync::mpsc, thread};

    use lsp_types::{
        DiagnosticSeverity, FileChangeType, FileEvent, TextDocumentIdentifier, WorkspaceFolder,
        notification::Initialized,
        request::{Initialize, Shutdown},
    };
    use serde_json::{Value, json};
    use zenith_diagnostics::{Columns, Span};
    use zenith_utils::{fs::adjust_canonicalisation, location::ByteRange};

    use super::*;

    /// A check that finds `count` violations in `web/a.ts`, an unused
    /// suppression if `unused`, and a finding in no file if `unlocated`,
    /// under a root in the temporary directory.
    fn found(count: usize, unused: bool, unlocated: bool) -> Option<Result<Found, Failure>> {
        let mut sources = SourceMap::with_root(&env::temp_dir());
        let source = sources.add("web/a.ts", "import './b'; // zenith-ignore a\n");
        let mut reports: Vec<_> = (0..count)
            .map(|_| {
                let mut report = Report::error("`web` must not import `api`").with_id("a");
                report.add_span(Span::new(source, ByteRange::new(7, 12)));
                report
            })
            .collect();
        if unlocated {
            reports.push(Report::error("`web/x` is not declared as a layer of `web`"));
        }
        let mut suppression = Report::warning("the suppression has no effect");
        suppression.add_span(Span::new(source, ByteRange::new(14, 32)));
        let unnecessary = if unused { vec![suppression] } else { Vec::new() };
        Some(Ok(Found { sources, reports, unnecessary }))
    }

    /// A check that fails on an error in the configuration file, which is
    /// shown on the file if `located`, and on an error that is in no file.
    fn invalid(located: bool) -> Option<Result<Found, Failure>> {
        let mut sources = SourceMap::with_root(&env::temp_dir());
        let source = sources.add("zenith.toml", "[files]\nexclud = []\n");
        let mut report = Report::error("unknown key `exclud`");
        if located {
            report.add_span(Span::new(source, ByteRange::new(8, 14)));
        }
        let unlocated = Report::error("there is no contract `x`");
        Some(Err(Failure::Reports(sources, vec![report, unlocated])))
    }

    /// A server on a connection to an editor that initialised it with
    /// `params`, and the editor's end of the connection.
    fn start(params: Value) -> (Server, Connection) {
        let (server, editor) = Connection::memory();
        let initialize = Request::new(RequestId::from(1), Initialize::METHOD.into(), params);
        editor.sender.send(initialize.into()).unwrap();
        let initialized = Notification::new(Initialized::METHOD.into(), json!({}));
        editor.sender.send(initialized.into()).unwrap();
        let server = Server::new(server).unwrap();
        let Message::Response(response) = editor.receiver.recv().unwrap() else {
            panic!("expected the response to `initialize`");
        };
        assert_eq!(
            response.response_result.unwrap()["capabilities"]["textDocumentSync"]["save"],
            true
        );
        (server, editor)
    }

    /// The next message from the server, which must be a notification of
    /// `method`, and its parameters.
    fn notified(editor: &Connection, method: &str) -> Value {
        match editor.receiver.recv().unwrap() {
            Message::Notification(notification) if notification.method == method => {
                notification.params
            }
            message => panic!("expected `{method}`, got {message:?}"),
        }
    }

    /// The file of the next diagnostics that the server publishes, relative
    /// to the temporary directory, and their severities.
    fn published(editor: &Connection) -> (String, Vec<DiagnosticSeverity>) {
        let params: PublishDiagnosticsParams =
            serde_json::from_value(notified(editor, PublishDiagnostics::METHOD)).unwrap();
        let path = params.uri.to_file_path().unwrap();
        let file = path.strip_prefix(env::temp_dir()).unwrap().to_string_lossy().replace('\\', "/");
        (file, params.diagnostics.iter().map(|diagnostic| diagnostic.severity.unwrap()).collect())
    }

    /// The kind of the next message that the server shows, and the message.
    fn shown(editor: &Connection) -> (MessageType, String) {
        let params: ShowMessageParams =
            serde_json::from_value(notified(editor, ShowMessage::METHOD)).unwrap();
        (params.typ, params.message)
    }

    /// Tell the server that a file has been saved.
    fn save(editor: &Connection) {
        let uri = Url::from_file_path(env::temp_dir().join("web/a.ts")).unwrap();
        let params =
            DidSaveTextDocumentParams { text_document: TextDocumentIdentifier { uri }, text: None };
        let notification = Notification::new(DidSaveTextDocument::METHOD.into(), params);
        editor.sender.send(notification.into()).unwrap();
    }

    /// Tell the server to exit.
    fn exit(editor: &Connection) {
        editor.sender.send(Notification::new(Exit::METHOD.into(), json!(null)).into()).unwrap();
    }

    #[test]
    fn publishes_on_start_and_on_save_until_shut_down() {
        // The editor initialises the server, which agrees on UTF-8.
        let folder = Url::from_directory_path(env::temp_dir()).unwrap();
        let (server, editor) = start(json!({
            "capabilities": { "general": { "positionEncodings": ["utf-8"] } },
            "workspaceFolders": [WorkspaceFolder { uri: folder, name: "tmp".into() }],
        }));
        assert_eq!(server.editor.columns, Columns::Utf8);
        assert_eq!(server.folder(), Some(env::temp_dir().as_path()));

        // Its checks find two violations, fail four times, once by panicking, then
        // find a violation, an unused suppression and a finding in no file, twice,
        // and then nothing.
        let mut checks = [
            found(2, false, false),
            invalid(true),
            Some(Err(Failure::Message("the project could not be checked".into()))),
            invalid(false),
            None,
            found(1, true, true),
            found(1, true, true),
            found(0, false, false),
        ]
        .into_iter();
        let check = move |_| checks.next().unwrap().unwrap_or_else(|| panic!("the check broke"));
        let running = thread::spawn(move || server.run(check));
        let error = DiagnosticSeverity::ERROR;
        assert_eq!(published(&editor), ("web/a.ts".into(), vec![error, error]));

        // A failure with a location is shown there, and keeps the diagnostics of
        // the other files. Its reports that are in no file are shown as a message.
        save(&editor);
        assert_eq!(published(&editor), ("zenith.toml".into(), vec![error]));
        let no_contract = "error: there is no contract `x`";
        assert_eq!(shown(&editor), (MessageType::ERROR, no_contract.into()));
        // A failure without one is shown as a message, and restores the files that
        // showed the last failure.
        save(&editor);
        assert_eq!(published(&editor), ("zenith.toml".into(), vec![]));
        let message = "the project could not be checked";
        assert_eq!(shown(&editor), (MessageType::ERROR, message.into()));
        save(&editor);
        let message = format!("error: unknown key `exclud`\n{no_contract}");
        assert_eq!(shown(&editor), (MessageType::ERROR, message));
        // So is a check that panics, which leaves the server running.
        save(&editor);
        let message = "zenith crashed while checking the project: the check broke; the \
                       server's log has the details";
        assert_eq!(shown(&editor), (MessageType::ERROR, message.into()));

        // Unused suppressions are hints. Findings in no file are counted in a
        // message.
        save(&editor);
        let hint = DiagnosticSeverity::HINT;
        assert_eq!(published(&editor), ("web/a.ts".into(), vec![error, hint]));
        let message = "1 finding has no place in a file to show it; run `zenith check` to see it";
        assert_eq!(shown(&editor), (MessageType::WARNING, message.into()));
        // A check that finds the same sends nothing, not even the message. A file
        // that no longer has diagnostics is cleared.
        save(&editor);
        save(&editor);
        assert_eq!(published(&editor), ("web/a.ts".into(), vec![]));

        // Other requests are refused, and the editor shuts the server down.
        let hover = Request::new(RequestId::from(2), "textDocument/hover".into(), json!({}));
        editor.sender.send(hover.into()).unwrap();
        let Message::Response(response) = editor.receiver.recv().unwrap() else {
            panic!("expected the response to `textDocument/hover`");
        };
        assert_eq!(response.response_result.unwrap_err().code, ErrorCode::MethodNotFound as i32);
        let shutdown = Request::new(RequestId::from(3), Shutdown::METHOD.into(), json!(null));
        editor.sender.send(shutdown.into()).unwrap();
        exit(&editor);
        assert!(running.join().unwrap().unwrap());
    }

    #[test]
    fn shows_why_a_check_failed_after_what_the_last_found_in_a_file() {
        // The configuration file has a warning of its own, such as an entry that
        // matches nothing.
        let stale = || {
            let mut sources = SourceMap::with_root(&env::temp_dir());
            let source = sources.add("zenith.toml", "[files]\nexclud = []\n");
            let mut report = Report::warning("the entry matches nothing");
            report.add_span(Span::new(source, ByteRange::new(0, 7)));
            Some(Ok(Found { sources, reports: vec![report], ..Found::default() }))
        };
        let (server, editor) = start(json!({ "capabilities": {} }));
        let mut checks = [stale(), invalid(true), stale()].into_iter();
        let running = thread::spawn(move || server.run(move |_| checks.next().unwrap().unwrap()));
        let (error, warning) = (DiagnosticSeverity::ERROR, DiagnosticSeverity::WARNING);
        assert_eq!(published(&editor), ("zenith.toml".into(), vec![warning]));

        // The failure is shown after the warning, and the warning alone once a
        // check runs again.
        save(&editor);
        assert_eq!(published(&editor), ("zenith.toml".into(), vec![warning, error]));
        assert_eq!(shown(&editor).0, MessageType::ERROR);
        save(&editor);
        assert_eq!(published(&editor), ("zenith.toml".into(), vec![warning]));

        exit(&editor);
        assert!(!running.join().unwrap().unwrap());
    }

    #[test]
    fn tells_each_check_what_changed_while_the_editor_reports_it() {
        // The editor can report the changes to files, so the server asks it to.
        let folder = Url::from_directory_path(env::temp_dir()).unwrap();
        let (server, editor) = start(json!({
            "capabilities": { "workspace": { "didChangeWatchedFiles": { "dynamicRegistration": true } } },
            "rootUri": folder,
        }));
        let Message::Request(request) = editor.receiver.recv().unwrap() else {
            panic!("expected the request to watch files");
        };
        assert_eq!(request.method, RegisterCapability::METHOD);
        let params: RegistrationParams = serde_json::from_value(request.params).unwrap();
        assert_eq!(params.registrations[0].method, DidChangeWatchedFiles::METHOD);

        // Each check says what it was told.
        let (told, changes) = mpsc::channel();
        let running = thread::spawn(move || {
            server.run(move |changes| {
                told.send(changes).unwrap();
                found(0, false, false).unwrap()
            })
        });
        assert_eq!(changes.recv().unwrap(), Changes::Unknown);

        // A save is checked with the files that changed since the last check, which
        // are named as zenith names them, and the file that was saved.
        let root = adjust_canonicalisation(env::temp_dir());
        let changed =
            |names: &[&str]| Changes::Files(names.iter().map(|name| root.join(name)).collect());
        let watched = |names: &[&str]| {
            let changes = names
                .iter()
                .map(|name| {
                    let uri = Url::from_file_path(env::temp_dir().join(name)).unwrap();
                    FileEvent::new(uri, FileChangeType::CHANGED)
                })
                .collect();
            let params = DidChangeWatchedFilesParams { changes };
            Notification::new(DidChangeWatchedFiles::METHOD.into(), params).into()
        };
        editor.sender.send(watched(&["web/b.ts", "web/c.ts"])).unwrap();
        save(&editor);
        assert_eq!(changes.recv().unwrap(), changed(&["web/a.ts", "web/b.ts", "web/c.ts"]));
        save(&editor);
        assert_eq!(changes.recv().unwrap(), changed(&["web/a.ts"]));

        // A report that can't be read, or a save that doesn't say what was saved,
        // could be of any change.
        let invalid = Notification::new(DidChangeWatchedFiles::METHOD.into(), json!({}));
        editor.sender.send(invalid.into()).unwrap();
        save(&editor);
        assert_eq!(changes.recv().unwrap(), Changes::Unknown);
        let unsaid = Notification::new(DidSaveTextDocument::METHOD.into(), json!({}));
        editor.sender.send(unsaid.into()).unwrap();
        assert_eq!(changes.recv().unwrap(), Changes::Unknown);
        save(&editor);
        assert_eq!(changes.recv().unwrap(), changed(&["web/a.ts"]));

        // Once the editor refuses to report them, any change could have been.
        let refused = Response::new_err(request.id, ErrorCode::InvalidParams as i32, "no".into());
        editor.sender.send(refused.into()).unwrap();
        editor.sender.send(watched(&["web/b.ts"])).unwrap();
        save(&editor);
        assert_eq!(changes.recv().unwrap(), Changes::Unknown);

        exit(&editor);
        assert!(!running.join().unwrap().unwrap());
    }

    #[test]
    fn exits_without_being_shut_down() {
        // An editor that predates workspace folders names its root.
        let root = Url::from_directory_path(env::temp_dir()).unwrap();
        let (server, editor) = start(json!({ "capabilities": {}, "rootUri": root }));
        assert_eq!(server.editor.columns, Columns::Utf16);
        assert_eq!(server.folder(), Some(env::temp_dir().as_path()));

        let running = thread::spawn(move || server.run(|_| found(0, false, false).unwrap()));
        exit(&editor);
        assert!(!running.join().unwrap().unwrap());
    }
}
