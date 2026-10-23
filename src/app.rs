//! The Gaza Man window: workspace state, actions and the frame loop.
//! The panels themselves live in `ui_sidebar.rs`, `ui_request.rs` and `ui_dialogs.rs`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, FontId, Key, Modifiers};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::codegen::Lang;
use crate::collection::{
    self, Assertion, Auth, BodyType, Capture, Collection, Environment, Folder, HistoryEntry, Id,
    KeyValue, Node, Op, Request, Scope, Source, new_id,
};
use crate::export::Format;
use crate::http::{self, CookieJar, Outcome, Prepared, Settings};
use crate::import::{self, Imported};
use crate::lens::Lens;
use crate::tests::{self, TestResult};
use crate::theme;

const STORAGE_KEY: &str = "workspace";
/// Where version 1 kept its single collection.
const OLD_STORAGE_KEY: &str = "collection";
pub const METHODS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SidebarView {
    #[default]
    Collections,
    Environments,
    History,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    pub coll: Id,
    pub req: Id,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TabKind {
    Request {
        draft: Request,
        origin: Option<Origin>,
    },
    Collection {
        coll: Id,
    },
    Folder {
        coll: Id,
        folder: Id,
    },
    Environment {
        env: Id,
    },
    Globals,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tab {
    pub id: Id,
    pub kind: TabKind,
}

impl Tab {
    pub fn new(kind: TabKind) -> Self {
        Self { id: new_id(), kind }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Workspace {
    pub collections: Vec<Collection>,
    pub environments: Vec<Environment>,
    pub globals: Vec<KeyValue>,
    pub active_env: Option<Id>,
    pub history: Vec<HistoryEntry>,
    pub settings: Settings,
    pub cookies: CookieJar,
    pub dark: bool,
    pub tabs: Vec<Tab>,
    pub active_tab: Option<Id>,
    pub open_nodes: HashSet<Id>,
    pub code_lang: Lang,
    pub show_code: bool,
    pub sidebar_view: SidebarView,
    pub show_sidebar: bool,
    /// What's shown: Simple, Full, or a custom pick.
    pub features: crate::features::Features,
    /// Secrets for `pm.vault` and `{{vault:name}}`. Kept here so they never leave in an export.
    pub vault: Vec<KeyValue>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            collections: vec![],
            environments: vec![],
            globals: vec![],
            active_env: None,
            history: vec![],
            settings: Settings::default(),
            cookies: CookieJar::default(),
            dark: true,
            tabs: vec![],
            active_tab: None,
            open_nodes: HashSet::new(),
            code_lang: Lang::Curl,
            show_code: false,
            sidebar_view: SidebarView::Collections,
            show_sidebar: true,
            features: crate::features::Features::default(),
            vault: vec![],
        }
    }
}

/// A first-run workspace with a demo collection on real public APIs (dummyjson.com, httpbin,
/// Open-Meteo, the Countries GraphQL API and Postman's echo servers). The folders are numbered
/// like a lesson: from a first GET to tokens, scripts and live connections.
pub fn demo_workspace() -> Workspace {
    let mut c = Collection::new("Gaza Man Demo");
    c.description = "A tour of Gaza Man on real public APIs, from a first GET to tokens, GraphQL and live connections. \
                     Open the folders in order: each request's Docs tab says what it shows."
        .into();
    c.variables.push(KeyValue::new("baseUrl", "https://dummyjson.com"));

    let test = |source, path: &str, op, expected: &str| Assertion {
        enabled: true,
        source,
        path: path.into(),
        op,
        expected: expected.into(),
    };
    let req = |name: &str, method: &str, url: &str, doc: &str| {
        let mut r = Request::new(name);
        r.method = method.into();
        r.url = url.into();
        r.sync_from_url();
        r.description = doc.into();
        r
    };
    let json = |r: &mut Request, body: &str| {
        r.body_type = BodyType::Raw;
        r.raw = body.into();
    };
    let folder = |name: &str, items: Vec<Request>| Folder {
        name: name.into(),
        items: items.into_iter().map(Node::Request).collect(),
        ..Default::default()
    };

    // 1. Reading data
    let mut list = req("List products", "GET", "{{baseUrl}}/products?limit=5&select=title,price",
        "A GET request reads data. The query params (limit, select) show up in the Params tab: change them and Send again.");
    list.tests = vec![
        test(Source::Status, "", Op::Equals, "200"),
        test(Source::Json, "products.length", Op::Equals, "5"),
    ];
    let mut one = req("One product", "GET", "{{baseUrl}}/products/:id",
        "`:id` in the URL is a path variable: set its value in the Params tab.");
    one.path_vars[0].value = "1".into();
    one.tests = vec![test(Source::Json, "id", Op::Equals, "1")];
    let search = req("Search", "GET", "{{baseUrl}}/products/search?q=phone&limit=3",
        "Searching with a query param. Try another word instead of phone.");
    let weather = req("Weather in Gaza", "GET",
        "https://api.open-meteo.com/v1/forecast?latitude=31.50&longitude=34.47&current_weather=true",
        "A different public API (Open-Meteo): the current weather for Gaza's coordinates.");

    // 2. Sending data
    let mut add = req("Add a product", "POST", "{{baseUrl}}/products/add",
        "POST sends data in the Body (raw JSON here). A new resource answers 201 Created.");
    json(&mut add, "{\n  \"title\": \"Gaza olive oil\",\n  \"price\": 12\n}");
    add.tests = vec![test(Source::Status, "", Op::Equals, "201"), test(Source::Json, "id", Op::Exists, "")];
    let mut update = req("Replace a product", "PUT", "{{baseUrl}}/products/1",
        "PUT sends the new version of a resource.");
    json(&mut update, "{\n  \"title\": \"Updated title\",\n  \"price\": 15\n}");
    let mut patch = req("Change one field", "PATCH", "{{baseUrl}}/products/1",
        "PATCH sends only the fields that change.");
    json(&mut patch, "{\n  \"price\": 9.5\n}");
    let mut delete = req("Delete a product", "DELETE", "{{baseUrl}}/products/1",
        "DELETE removes a resource. This API answers with the deleted product and isDeleted: true.");
    delete.tests = vec![test(Source::Json, "isDeleted", Op::Equals, "true")];
    let mut form = req("Send a form", "POST", "https://httpbin.org/post",
        "form-data, like an HTML form. httpbin answers with what it received.");
    form.body_type = BodyType::FormData;
    form.form = vec![KeyValue::new("name", "Gaza Man"), KeyValue::new("city", "Gaza")];
    form.tests = vec![test(Source::Json, "form.city", Op::Equals, "Gaza")];

    // 3. Logging in
    let mut login = req("Log in", "POST", "{{baseUrl}}/auth/login",
        "Logs in and saves the accessToken from the response into {{token}} (see the Variables tab). The next request uses it.");
    json(&mut login, "{\n  \"username\": \"emilys\",\n  \"password\": \"emilyspass\"\n}");
    login.tests = vec![test(Source::Status, "", Op::Equals, "200")];
    login.captures = vec![Capture { var: "token".into(), path: "accessToken".into(), ..Default::default() }];
    let mut me = req("Who am I?", "GET", "{{baseUrl}}/auth/me",
        "Sends the saved token as a Bearer token (Authorization tab). Send \"Log in\" first.");
    me.auth = Auth::Bearer { token: "{{token}}".into() };
    me.tests = vec![test(Source::Json, "username", Op::Equals, "emilys")];

    // 4. Images and pages
    let image = req("An image", "GET", "https://flagcdn.com/w640/ps.png",
        "The response is a PNG (the flag of Palestine): the Body shows it as a picture.");
    let page = req("A web page", "GET", "https://httpbin.org/html",
        "An HTML page, shown as a readable page. \"Open in browser\" shows the real one.");
    let dog = req("Random dog", "GET", "https://dog.ceo/api/breeds/image/random",
        "JSON with a link to a random dog photo. Copy the link into a new request to see the photo.");

    // 5. GraphQL
    let mut gql = req("Country (GraphQL)", "POST", "https://countries.trevorblades.com/graphql",
        "GraphQL: one URL, and the query says which fields come back. Press Fetch schema for suggestions as you type.");
    gql.body_type = BodyType::GraphQl;
    gql.gql_query = "query Country($code: ID!) {\n  country(code: $code) {\n    name\n    native\n    capital\n    emoji\n    currency\n  }\n}".into();
    gql.gql_variables = "{\n  \"code\": \"PS\"\n}".into();
    gql.tests = vec![test(Source::Json, "data.country.name", Op::Equals, "Palestine")];

    // 6. Live connections
    let mut ws = req("WebSocket echo", "GET", "wss://ws.postman-echo.com/raw",
        "Connect, then send a message: the server sends it straight back. The connection stays open.");
    ws.kind = collection::RequestKind::WebSocket;
    ws.ws_message = "Hello from Gaza Man!".into();
    let mut sio = req("Socket.IO echo", "GET", "https://ws.postman-echo.com/socketio",
        "Connect: the server greets you and sends random events. Emit an event and it comes back.");
    sio.kind = collection::RequestKind::SocketIo;
    sio.sio_event = "chat".into();
    sio.ws_message = "{\n  \"text\": \"Hello from class\"\n}".into();

    // 7. Scripts
    let mut scripted = req("Tests in JavaScript", "GET", "{{baseUrl}}/products/1",
        "The Scripts tab: a pre-request script adds a header, and a post-response script checks the result with pm.test.");
    crate::script::set_code(&mut scripted.events, "prerequest",
        "// Runs before the request is sent\npm.request.headers.add({ key: \"X-Lesson\", value: \"scripts\" });\nconsole.log(\"Sending\", pm.variables.replaceIn(pm.request.url.toString()));");
    crate::script::set_code(&mut scripted.events, "test",
        "// Runs after the response arrives\npm.test(\"Status code is 200\", () => {\n    pm.response.to.have.status(200);\n});\n\nconst product = pm.response.json();\npm.test(\"Has a title and a price\", () => {\n    pm.expect(product).to.have.property(\"title\");\n    pm.expect(product.price).to.be.a(\"number\").and.above(0);\n});\n\npm.environment.set(\"productTitle\", product.title);\nconsole.log(\"Saved\", product.title);");

    let mut visual = req("Products as a table", "GET", "{{baseUrl}}/products?limit=6&select=title,price,rating",
        "pm.visualizer turns a JSON response into an HTML table: see the Visualize tab of the response.");
    crate::script::set_code(&mut visual.events, "test",
        "// Draw the response as a table (Handlebars template)\nconst template = `\n<h2>{{products.length}} products</h2>\n<table>\n  <tr><th>#</th><th>Product</th><th>Price</th><th>Rating</th></tr>\n  {{#each products}}\n  <tr><td>{{@index}}</td><td>{{title}}</td><td>{{price}} USD</td><td>{{rating}}</td></tr>\n  {{/each}}\n</table>`;\n\npm.visualizer.set(template, pm.response.json());");

    let folders = vec![
        folder("1. Reading data", vec![list, one, search, weather]),
        folder("2. Sending data", vec![add, update, patch, delete, form]),
        folder("3. Logging in", vec![login, me]),
        folder("4. Images and pages", vec![image, page, dog]),
        folder("5. GraphQL", vec![gql]),
        folder("6. Live connections", vec![ws, sio]),
        folder("7. Scripts", vec![scripted, visual]),
    ];
    let mut open: HashSet<Id> = [c.id].into_iter().collect();
    open.insert(folders[0].id);
    c.items = folders.into_iter().map(Node::Folder).collect();

    let env = Environment {
        name: "Demo".into(),
        values: vec![KeyValue::new("token", "")],
        ..Default::default()
    };
    Workspace {
        active_env: Some(env.id),
        collections: vec![c],
        environments: vec![env],
        open_nodes: open,
        ..Default::default()
    }
}

/// What a sidebar item or menu entry points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Collection(Id),
    /// Folder or request inside a collection.
    Node(Id, Id),
    Environment(Id),
    History(Id),
}

pub enum PromptKind {
    NewCollection,
    NewFolder { coll: Id, parent: Option<Id> },
    NewEnvironment,
    Rename(Target),
}

pub enum ConfirmKind {
    Delete(Target),
    ClearHistory,
}

pub enum Dialog {
    Prompt {
        title: String,
        text: String,
        kind: PromptKind,
    },
    Confirm {
        title: String,
        message: String,
        kind: ConfirmKind,
    },
    SaveAs {
        tab: Id,
        name: String,
        coll: Option<Id>,
        folder: Option<Id>,
        close_after: bool,
    },
    CloseDirty {
        tab: Id,
    },
    Import {
        text: String,
        error: Option<String>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

pub struct Toast {
    pub text: String,
    pub kind: ToastKind,
    pub at: Instant,
    /// Show an "Undo" button.
    pub undo: bool,
}

/// What undo / redo restores: the collections and variables, not the open tabs.
#[derive(Clone, PartialEq)]
pub struct Snapshot {
    collections: Vec<Collection>,
    environments: Vec<Environment>,
    globals: Vec<KeyValue>,
    active_env: Option<Id>,
}

pub struct UndoStack {
    past: Vec<Snapshot>,
    future: Vec<Snapshot>,
    /// The state the last step was taken from.
    base: Snapshot,
    changed_at: Option<Instant>,
    /// Bumped on every recorded change (the mock server watches it).
    pub revision: u64,
}

impl Snapshot {
    fn of(ws: &Workspace) -> Self {
        Self {
            collections: ws.collections.clone(),
            environments: ws.environments.clone(),
            globals: ws.globals.clone(),
            active_env: ws.active_env,
        }
    }

    fn same_as(&self, ws: &Workspace) -> bool {
        self.active_env == ws.active_env
            && self.globals == ws.globals
            && self.environments == ws.environments
            && self.collections == ws.collections
    }

    fn restore(self, ws: &mut Workspace) {
        ws.collections = self.collections;
        ws.environments = self.environments;
        ws.globals = self.globals;
        ws.active_env = self.active_env;
    }
}

/// What the UI asks for. Collected while drawing and carried out at the end of the frame,
/// so the panels never fight over borrows.
pub enum Action {
    Send(Id),
    Cancel(Id),
    Save(Id),
    CloseTab(Id),
    ForceCloseTab(Id),
    CloseOtherTabs(Id),
    FocusTab(Id),
    NewRequestTab,
    NewTyped(NewKind),
    /// A fresh copy of the demo collection.
    AddDemo,
    DuplicateTab(Id),
    OpenRequest {
        coll: Id,
        req: Id,
    },
    OpenTab(TabKind),
    OpenHistory(Id),
    NewCollection,
    NewEnvironment,
    AddRequest {
        coll: Id,
        folder: Option<Id>,
    },
    AddFolder {
        coll: Id,
        parent: Option<Id>,
    },
    Rename(Target),
    Delete(Target),
    Duplicate(Target),
    Shift {
        coll: Id,
        node: Id,
        delta: isize,
    },
    Move {
        from: Id,
        node: Id,
        to: Id,
        drop: DropAt,
    },
    Export(Id, Format),
    ExportEnvironment(Id),
    Import,
    ImportFiles(Vec<PathBuf>),
    Run {
        coll: Id,
        folder: Option<Id>,
    },
    FetchToken(Id),
    /// Ask the GraphQL server of this tab for its schema.
    FetchSchema(Id),
    /// WebSocket / Socket.IO: open or close the connection of a tab, or send its message.
    SocketConnect(Id),
    SocketDisconnect(Id),
    SocketSend(Id),
    SocketClear(Id),
    Captures(Option<Id>, Vec<(Scope, String, String)>),
    SetActiveEnv(Option<Id>),
    SaveResponse(Id),
    Notify(String, ToastKind),
    Undo,
    Redo,
    StartMock(Id),
    StopMock(Id),
    /// Move a tab in the tab bar to just before another one.
    MoveTab { tab: Id, before: Id },
}

/// The kinds of request the New menu makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewKind {
    Http,
    GraphQl,
    WebSocket,
    SocketIo,
}

/// Where a dragged sidebar item lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAt {
    /// At the end of a folder (or of the collection, for `None`).
    Into(Option<Id>),
    Before(Id),
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ReqTab {
    #[default]
    Params,
    Auth,
    Headers,
    Body,
    Tests,
    Capture,
    Examples,
    Scripts,
    Docs,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ResTab {
    #[default]
    Body,
    /// `pm.visualizer.set` output.
    Visualize,
    Cookies,
    Headers,
    Tests,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum BodyMode {
    #[default]
    Pretty,
    Raw,
    Tree,
    /// Images, and a reader view of HTML pages.
    Preview,
}

/// A finished response, with the body already formatted for display.
pub struct ResponseView {
    pub outcome: Outcome,
    pub display: String,
    pub json: Option<Value>,
    pub tests: Vec<TestResult>,
    pub captured: Vec<(Scope, String, String)>,
    pub cookies: Vec<http::Cookie>,
    pub preview: crate::preview::Kind,
    /// Decoded once when the response arrives, uploaded to the GPU on first display.
    pub image: Option<Result<crate::preview::DecodedImage, String>>,
    pub texture: std::cell::OnceCell<egui::TextureHandle>,
    pub html: std::cell::OnceCell<egui::text::LayoutJob>,
    /// What the pre-request and post-response scripts did.
    pub script: Option<crate::script::Report>,
}

impl ResponseView {
    fn new(
        outcome: Outcome,
        sent: Prepared,
        tests: Vec<TestResult>,
        captured: Vec<(Scope, String, String)>,
    ) -> Self {
        let (display, json) = match &outcome {
            Outcome::Response { body, .. } => match serde_json::from_str::<Value>(body) {
                Ok(v) if !body.trim().is_empty() => (
                    serde_json::to_string_pretty(&v).unwrap_or_default(),
                    Some(v),
                ),
                _ => (body.clone(), None),
            },
            Outcome::Error { message, .. } => (message.clone(), None),
        };
        let (_, host, path) = http::split_url(&sent.url);
        let cookies = match &outcome {
            Outcome::Response { headers, .. } => headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                .filter_map(|(_, v)| http::parse_set_cookie(v, &host, &path))
                .collect(),
            Outcome::Error { .. } => vec![],
        };
        let preview = match &outcome {
            Outcome::Response { bytes, .. } => crate::preview::kind(outcome.header("content-type").unwrap_or(""), bytes),
            Outcome::Error { .. } => crate::preview::Kind::None,
        };
        let image = match (&outcome, preview) {
            (Outcome::Response { bytes, .. }, crate::preview::Kind::Image) => Some(crate::preview::decode_image(bytes)),
            _ => None,
        };
        // Binary image bytes as text are just noise; keep a short description instead.
        let display = match &image {
            Some(Ok(img)) => format!("({} image, {} × {} px)", img.format, img.image.size[0], img.image.size[1]),
            Some(Err(e)) => format!("({e})"),
            None => display,
        };
        Self {
            outcome,
            display: crate::rtl::shape(&display),
            json,
            tests,
            captured,
            cookies,
            preview,
            image,
            texture: Default::default(),
            html: Default::default(),
            script: None,
        }
    }
}

pub struct ConsoleEntry {
    pub at: u64,
    pub sent: Prepared,
    pub outcome: Outcome,
    /// What scripts printed with `console.log` (and their errors) around this request.
    pub logs: Vec<crate::script::LogLine>,
}

#[derive(Clone)]
pub struct RunResult {
    pub iteration: u32,
    pub name: String,
    pub method: String,
    pub url: String,
    pub status: Option<u16>,
    pub time_ms: u128,
    pub size: usize,
    pub tests: Vec<TestResult>,
    pub error: Option<String>,
}

impl RunResult {
    pub fn passed(&self) -> bool {
        self.error.is_none()
            && self.status.is_some_and(|s| s < 400)
            && self.tests.iter().all(|t| t.passed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum RunFilter {
    #[default]
    All,
    Passed,
    Failed,
}

pub struct Runner {
    pub open: bool,
    pub coll: Option<Id>,
    pub folder: Option<Id>,
    pub iterations: u32,
    pub delay_ms: u64,
    pub stop_on_fail: bool,
    pub running: bool,
    pub stop: Arc<AtomicBool>,
    pub results: Vec<RunResult>,
    pub total: usize,
    pub started: Option<Instant>,
    pub elapsed: Duration,
    pub filter: RunFilter,
    pub expanded: HashSet<usize>,
    /// A CSV / JSON data file: one iteration per row.
    pub data_path: String,
    pub data: crate::data::Rows,
}

impl Default for Runner {
    fn default() -> Self {
        Self {
            open: false,
            coll: None,
            folder: None,
            iterations: 1,
            delay_ms: 0,
            stop_on_fail: false,
            running: false,
            stop: Arc::new(AtomicBool::new(false)),
            results: vec![],
            total: 0,
            started: None,
            elapsed: Duration::ZERO,
            filter: RunFilter::All,
            expanded: HashSet::new(),
            data_path: String::new(),
            data: vec![],
        }
    }
}

enum Msg {
    Done {
        tab: Id,
        seq: u64,
        coll: Option<Id>,
        origin: Option<Id>,
        request: Box<Request>,
        sent: Prepared,
        outcome: Outcome,
        report: Option<Box<crate::script::Report>>,
    },
    RunResult(
        RunResult,
        Option<Id>,
        Vec<(Scope, String, String)>,
        Box<(Prepared, Outcome, crate::script::Report)>,
    ),
    RunFinished,
    Token {
        tab: Id,
        result: Result<String, String>,
    },
    Schema {
        tab: Id,
        result: Result<crate::graphql::Schema, String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SocketState {
    Connecting,
    Open,
    Closed,
}

/// A WebSocket or Socket.IO connection and everything said on it.
pub struct SocketView {
    pub session: Option<crate::ws::Session>,
    pub state: SocketState,
    /// Why it closed, or what went wrong.
    pub reason: String,
    pub url: String,
    pub handshake: Vec<(String, String)>,
    pub log: Vec<crate::ws::Message>,
    pub filter: String,
}

pub struct GazaMan {
    pub ws: Workspace,
    pub cookies: Arc<Mutex<CookieJar>>,
    pub responses: HashMap<Id, ResponseView>,
    /// Tab id -> (send sequence, started).
    pub pending: HashMap<Id, (u64, Instant)>,
    seq: u64,
    pub last_status: HashMap<Id, Option<u16>>,
    pub req_tabs: HashMap<Id, ReqTab>,
    pub res_tab: ResTab,
    pub body_mode: BodyMode,
    pub res_search: String,
    pub sidebar_search: String,
    pub focus_search: bool,
    pub actions: Vec<Action>,
    pub dialog: Option<Dialog>,
    pub runner: Runner,
    pub show_settings: bool,
    pub show_cookies: bool,
    pub show_console: bool,
    pub console: Vec<ConsoleEntry>,
    pub bulk_edit: HashMap<String, String>,
    pub show_auto_headers: bool,
    pub fetching_token: HashSet<Id>,
    /// GraphQL schemas by tab, for autocomplete and the schema explorer.
    pub gql_schemas: HashMap<Id, Result<crate::graphql::Schema, String>>,
    pub fetching_schema: HashSet<Id>,
    pub sockets: HashMap<Id, SocketView>,
    /// Which script the Scripts editors show.
    pub script_phase: crate::script::Phase,
    pub toast: Option<Toast>,
    pub lens: Lens,
    pub undo: UndoStack,
    pub body_find: String,
    /// Running mock servers, by collection.
    pub mocks: HashMap<Id, crate::mock::MockServer>,
    pub mock_port: u16,
    pub mock_public: bool,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    /// Screenshot mode, for checking the UI from scripts: don't touch the saved workspace.
    shot: Option<crate::shot::Shot>,
}

impl GazaMan {
    pub fn new(cc: &eframe::CreationContext<'_>, preload: Option<PathBuf>) -> Self {
        let shot = crate::shot::Shot::from_env();
        let mut ws = if shot.is_some() {
            demo_workspace()
        } else {
            let saved = cc
                .storage
                .and_then(|s| s.get_string(STORAGE_KEY))
                .and_then(|s| serde_json::from_str::<Workspace>(&s).ok());
            match saved {
                Some(mut ws) => {
                    replace_old_demo(&mut ws);
                    ws
                }
                None => {
                    let mut ws = demo_workspace();
                    // Bring over the collection from version 1, if there is one.
                    let old = cc
                        .storage
                        .and_then(|s| s.get_string(OLD_STORAGE_KEY))
                        .and_then(|s| serde_json::from_str::<Value>(&s).ok());
                    if let Some(c) = old.and_then(|json| collection::parse(&json).ok()) {
                        ws.open_nodes.insert(c.id);
                        ws.collections.insert(0, c);
                    }
                    ws
                }
            }
        };
        ws.history.truncate(ws.settings.history_limit.max(1));
        theme::install(&cc.egui_ctx, ws.dark);

        let (tx, rx) = channel();
        let cookies = Arc::new(Mutex::new(ws.cookies.clone()));
        let undo_base = Snapshot::of(&ws);
        let mut app = Self {
            ws,
            cookies,
            responses: HashMap::new(),
            pending: HashMap::new(),
            seq: 0,
            last_status: HashMap::new(),
            req_tabs: HashMap::new(),
            res_tab: ResTab::Body,
            body_mode: BodyMode::Pretty,
            res_search: String::new(),
            sidebar_search: String::new(),
            focus_search: false,
            actions: vec![],
            dialog: None,
            runner: Runner::default(),
            show_settings: false,
            show_cookies: false,
            show_console: false,
            console: vec![],
            bulk_edit: HashMap::new(),
            show_auto_headers: false,
            fetching_token: HashSet::new(),
            gql_schemas: HashMap::new(),
            fetching_schema: HashSet::new(),
            sockets: HashMap::new(),
            script_phase: crate::script::Phase::PreRequest,
            toast: None,
            lens: Lens::default(),
            undo: UndoStack { past: vec![], future: vec![], base: undo_base, changed_at: None, revision: 0 },
            body_find: String::new(),
            mocks: HashMap::new(),
            mock_port: 3001,
            mock_public: false,
            tx,
            rx,
            shot,
        };
        if app.ws.tabs.is_empty() {
            if let Some((coll, req)) = app
                .ws
                .collections
                .first()
                .and_then(|c| c.requests().first().map(|r| (c.id, r.id)))
            {
                app.open_request(coll, req);
            }
        }
        if let Some(path) = preload {
            app.import_files(&[path]);
        }
        app
    }

    pub fn shot_mut(&mut self) -> Option<&mut crate::shot::Shot> {
        self.shot.as_mut()
    }

    pub fn notify(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toast = Some(Toast {
            text: text.into(),
            kind,
            at: Instant::now(),
            undo: false,
        });
    }

    /// A notice with an "Undo" button, after something destructive.
    pub fn notify_undo(&mut self, text: impl Into<String>) {
        self.notify(text, ToastKind::Info);
        if let Some(t) = &mut self.toast {
            t.undo = true;
        }
    }

    // -----------------------------------------------------------------------------------------
    // Undo / redo
    // -----------------------------------------------------------------------------------------

    /// Records an undo step when the workspace changed. Typing is grouped: a step is taken
    /// once the keyboard is free, or after a second.
    fn track_changes(&mut self, ctx: &egui::Context) {
        if self.undo.base.same_as(&self.ws) {
            self.undo.changed_at = None;
            return;
        }
        let since = *self.undo.changed_at.get_or_insert_with(Instant::now);
        if ctx.egui_wants_keyboard_input() && since.elapsed() < Duration::from_secs(1) {
            ctx.request_repaint_after(Duration::from_millis(300));
            return;
        }
        self.commit_change();
    }

    fn commit_change(&mut self) {
        if self.undo.base.same_as(&self.ws) {
            return;
        }
        let previous = std::mem::replace(&mut self.undo.base, Snapshot::of(&self.ws));
        self.undo.past.push(previous);
        if self.undo.past.len() > 60 {
            self.undo.past.remove(0);
        }
        self.undo.future.clear();
        self.undo.changed_at = None;
        self.undo.revision += 1;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.past.is_empty() || !self.undo.base.same_as(&self.ws)
    }

    // -----------------------------------------------------------------------------------------
    // Mock servers
    // -----------------------------------------------------------------------------------------

    pub fn start_mock(&mut self, ctx: &egui::Context, coll: Id) {
        let Some(c) = self.collection(coll) else { return };
        let routes = crate::mock::routes(c, &self.vars_for(Some(coll)));
        let taken: Vec<u16> = self.mocks.values().map(|m| m.port).collect();
        let mut port = self.mock_port;
        while taken.contains(&port) {
            port += 1;
        }
        match crate::mock::MockServer::start(coll, port, self.mock_public, routes, self.undo.revision, ctx.clone()) {
            Ok(m) => {
                let url = m.url();
                self.mocks.insert(coll, m);
                self.notify(format!("Mock server running at {url}"), ToastKind::Success);
            }
            Err(e) => self.notify(e, ToastKind::Error),
        }
    }

    pub fn stop_mock(&mut self, coll: Id) {
        if let Some(m) = self.mocks.remove(&coll) {
            m.stop();
        }
    }

    /// Keeps running mock servers in step with the saved collections.
    fn refresh_mocks(&mut self) {
        let revision = self.undo.revision;
        let stale: Vec<Id> = self.mocks.values().filter(|m| m.revision != revision).map(|m| m.coll).collect();
        for coll in stale {
            let Some(c) = self.collection(coll) else {
                self.stop_mock(coll);
                continue;
            };
            let routes = crate::mock::routes(c, &self.vars_for(Some(coll)));
            if let Some(m) = self.mocks.get_mut(&coll) {
                if let Ok(mut r) = m.routes.write() {
                    *r = routes;
                }
                m.revision = revision;
            }
        }
    }

    pub fn can_redo(&self) -> bool {
        !self.undo.future.is_empty()
    }

    pub fn undo(&mut self) {
        self.commit_change();
        let Some(previous) = self.undo.past.pop() else { return };
        self.undo.future.push(Snapshot::of(&self.ws));
        previous.clone().restore(&mut self.ws);
        self.undo.base = previous;
        self.undo.revision += 1;
        self.notify("Undone", ToastKind::Info);
    }

    pub fn redo(&mut self) {
        self.commit_change();
        let Some(next) = self.undo.future.pop() else { return };
        self.undo.past.push(Snapshot::of(&self.ws));
        next.clone().restore(&mut self.ws);
        self.undo.base = next;
        self.undo.revision += 1;
        self.notify("Redone", ToastKind::Info);
    }

    // -----------------------------------------------------------------------------------------
    // Lookups
    // -----------------------------------------------------------------------------------------

    pub fn collection(&self, id: Id) -> Option<&Collection> {
        self.ws.collections.iter().find(|c| c.id == id)
    }

    pub fn collection_mut(&mut self, id: Id) -> Option<&mut Collection> {
        self.ws.collections.iter_mut().find(|c| c.id == id)
    }

    pub fn active_env(&self) -> Option<&Environment> {
        self.ws
            .active_env
            .and_then(|id| self.ws.environments.iter().find(|e| e.id == id))
    }

    /// Variables in Postman's order of precedence: globals, then collection, then environment.
    pub fn vars_for(&self, coll: Option<Id>) -> Vec<KeyValue> {
        let mut vars = self.ws.globals.clone();
        if let Some(c) = coll.and_then(|id| self.collection(id)) {
            vars.extend(c.variables.iter().cloned());
        }
        if let Some(e) = self.active_env() {
            vars.extend(e.values.iter().cloned());
        }
        vars.extend(self.ws.vault.iter().filter(|kv| kv.enabled).map(|kv| KeyValue::new(&format!("vault:{}", kv.key), &kv.value)));
        vars
    }

    pub fn tab(&self, id: Id) -> Option<&Tab> {
        self.ws.tabs.iter().find(|t| t.id == id)
    }

    pub fn tab_index(&self, id: Id) -> Option<usize> {
        self.ws.tabs.iter().position(|t| t.id == id)
    }

    /// The saved version of a request tab, if it came from a collection that still has it.
    pub fn origin_request(&self, origin: Option<Origin>) -> Option<&Request> {
        let o = origin?;
        self.collection(o.coll)?.request(o.req)
    }

    pub fn is_dirty(&self, tab: &Tab) -> bool {
        match &tab.kind {
            TabKind::Request { draft, origin } => match self.origin_request(*origin) {
                Some(saved) => saved != draft,
                None => {
                    origin.is_none() && (!draft.url.is_empty() || draft.body_type != BodyType::None)
                }
            },
            _ => false,
        }
    }

    pub fn tab_title(&self, tab: &Tab) -> String {
        match &tab.kind {
            TabKind::Request { draft, .. } => draft.name.clone(),
            TabKind::Collection { coll } => self
                .collection(*coll)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Deleted collection".into()),
            TabKind::Folder { coll, folder } => {
                match self.collection(*coll).and_then(|c| c.find(*folder)) {
                    Some(n) => n.name().to_string(),
                    None => "Deleted folder".into(),
                }
            }
            TabKind::Environment { env } => self
                .ws
                .environments
                .iter()
                .find(|e| e.id == *env)
                .map(|e| e.name.clone())
                .unwrap_or_else(|| "Deleted environment".into()),
            TabKind::Globals => "Globals".into(),
        }
    }

    /// The auth a draft request ends up using, following its folders and collection.
    pub fn effective_auth(&self, draft: &Request, origin: Option<Origin>) -> Auth {
        match origin.and_then(|o| self.collection(o.coll)) {
            Some(c) if c.find(draft.id).is_some() => c.effective_auth(draft.id, &draft.auth),
            _ => match &draft.auth {
                Auth::Inherit => Auth::None,
                a => a.clone(),
            },
        }
    }

    pub fn prepare(&self, draft: &Request, origin: Option<Origin>) -> Prepared {
        let vars = self.vars_for(origin.map(|o| o.coll));
        http::prepare(draft, &self.effective_auth(draft, origin), &vars)
    }

    // -----------------------------------------------------------------------------------------
    // Tabs
    // -----------------------------------------------------------------------------------------

    pub fn focus(&mut self, id: Id) {
        self.ws.active_tab = Some(id);
    }

    pub fn open_request(&mut self, coll: Id, req: Id) {
        let existing = self.ws.tabs.iter().find(|t| matches!(&t.kind, TabKind::Request { origin: Some(o), .. } if o.coll == coll && o.req == req));
        if let Some(t) = existing {
            let id = t.id;
            self.focus(id);
            return;
        }
        let Some(r) = self.collection(coll).and_then(|c| c.request(req)).cloned() else {
            return;
        };
        let tab = Tab::new(TabKind::Request {
            draft: r,
            origin: Some(Origin { coll, req }),
        });
        self.push_tab(tab);
    }

    fn push_tab(&mut self, tab: Tab) {
        let id = tab.id;
        // Insert next to the current tab, like a browser.
        let at = self
            .ws
            .active_tab
            .and_then(|a| self.tab_index(a))
            .map(|i| i + 1)
            .unwrap_or(self.ws.tabs.len());
        self.ws.tabs.insert(at, tab);
        self.focus(id);
    }

    pub fn open_tab(&mut self, kind: TabKind) {
        let same = |a: &TabKind, b: &TabKind| match (a, b) {
            (TabKind::Collection { coll: x }, TabKind::Collection { coll: y }) => x == y,
            (TabKind::Folder { folder: x, .. }, TabKind::Folder { folder: y, .. }) => x == y,
            (TabKind::Environment { env: x }, TabKind::Environment { env: y }) => x == y,
            (TabKind::Globals, TabKind::Globals) => true,
            _ => false,
        };
        if let Some(t) = self.ws.tabs.iter().find(|t| same(&t.kind, &kind)) {
            let id = t.id;
            self.focus(id);
            return;
        }
        self.push_tab(Tab::new(kind));
    }

    pub fn new_request_tab(&mut self) {
        self.new_typed_tab(NewKind::Http);
    }

    /// A new request of a kind, set up so its main editor is ready to use.
    pub fn new_typed_tab(&mut self, kind: NewKind) {
        // Asking for a kind of request means wanting to see it, whatever the mode.
        let f = &mut self.ws.features;
        match kind {
            NewKind::GraphQl => f.graphql = true,
            NewKind::WebSocket | NewKind::SocketIo => f.sockets = true,
            NewKind::Http => {}
        }
        let mut draft = Request::default();
        match kind {
            NewKind::Http => {}
            NewKind::GraphQl => {
                draft.name = "GraphQL Request".into();
                draft.method = "POST".into();
                draft.body_type = BodyType::GraphQl;
                draft.gql_query = "query {\n  \n}".into();
            }
            NewKind::WebSocket => {
                draft.name = "WebSocket".into();
                draft.kind = collection::RequestKind::WebSocket;
            }
            NewKind::SocketIo => {
                draft.name = "Socket.IO".into();
                draft.kind = collection::RequestKind::SocketIo;
            }
        }
        let tab = Tab::new(TabKind::Request { draft, origin: None });
        let id = tab.id;
        self.push_tab(tab);
        if kind != NewKind::Http {
            self.req_tabs.insert(id, ReqTab::Body);
        }
    }

    fn close_tab(&mut self, id: Id) {
        let Some(i) = self.tab_index(id) else { return };
        self.ws.tabs.remove(i);
        // Dropping the session closes its connection.
        self.sockets.remove(&id);
        self.responses.remove(&id);
        self.pending.remove(&id);
        self.req_tabs.remove(&id);
        if self.ws.active_tab == Some(id) {
            let next = self
                .ws
                .tabs
                .get(i)
                .or_else(|| self.ws.tabs.get(i.saturating_sub(1)))
                .map(|t| t.id);
            self.ws.active_tab = next;
        }
    }

    fn request_close(&mut self, id: Id) {
        match self.tab(id) {
            Some(t) if self.is_dirty(t) => self.dialog = Some(Dialog::CloseDirty { tab: id }),
            Some(_) => self.close_tab(id),
            None => {}
        }
    }

    /// Writes a request tab back into its collection. Asks where to save new requests.
    pub fn save_tab(&mut self, id: Id, close_after: bool) {
        let Some(tab) = self.tab(id) else { return };
        let TabKind::Request { draft, origin } = &tab.kind else {
            return;
        };
        let (draft, origin) = (draft.clone(), *origin);
        match origin {
            Some(o)
                if self
                    .collection(o.coll)
                    .and_then(|c| c.request(o.req))
                    .is_some() =>
            {
                if let Some(r) = self
                    .collection_mut(o.coll)
                    .and_then(|c| c.request_mut(o.req))
                {
                    *r = draft;
                }
                if close_after {
                    self.close_tab(id);
                } else {
                    self.notify("Saved", ToastKind::Success);
                }
            }
            _ => {
                let coll = self.ws.collections.first().map(|c| c.id);
                self.dialog = Some(Dialog::SaveAs {
                    tab: id,
                    name: draft.name.clone(),
                    coll,
                    folder: None,
                    close_after,
                });
            }
        }
    }

    /// Finishes "Save as": puts the draft into a collection and links the tab to it.
    pub fn save_as(
        &mut self,
        tab: Id,
        name: String,
        coll: Id,
        folder: Option<Id>,
        close_after: bool,
    ) {
        let Some(i) = self.tab_index(tab) else { return };
        let TabKind::Request { draft, origin } = &mut self.ws.tabs[i].kind else {
            return;
        };
        // A copy of a saved request gets a fresh id so both can live in collections.
        if origin.is_some()
            || self
                .ws
                .collections
                .iter()
                .any(|c| c.find(draft.id).is_some())
        {
            draft.id = new_id();
        }
        draft.name = name;
        *origin = Some(Origin {
            coll,
            req: draft.id,
        });
        let node = Node::Request(draft.clone());
        let Some(c) = self.collection_mut(coll) else {
            return;
        };
        match c.items_mut(folder) {
            Some(items) => items.push(node),
            None => c.items.push(node),
        }
        self.ws.open_nodes.insert(coll);
        if let Some(f) = folder {
            self.ws.open_nodes.insert(f);
        }
        if close_after {
            self.close_tab(tab);
        } else {
            self.notify("Saved to collection", ToastKind::Success);
        }
    }

    // -----------------------------------------------------------------------------------------
    // Sending
    // -----------------------------------------------------------------------------------------

    pub fn send(&mut self, ctx: &egui::Context, tab_id: Id) {
        // For sockets "Send" connects first, and then sends the message.
        if let Some(TabKind::Request { draft, .. }) = self.tab(tab_id).map(|t| &t.kind) {
            if draft.kind != collection::RequestKind::Http {
                match self.sockets.get(&tab_id).map(|s| s.state) {
                    Some(SocketState::Open) => self.socket_send(tab_id),
                    Some(SocketState::Connecting) => {}
                    _ => self.socket_connect(ctx, tab_id),
                }
                return;
            }
        }
        let Some(tab) = self.tab(tab_id) else { return };
        let TabKind::Request { draft, origin } = &tab.kind else {
            return;
        };
        let (draft, origin) = (&draft.clone(), *origin);
        let coll = origin.map(|o| o.coll);
        let origin_req = origin.map(|o| o.req);
        let chain = crate::script::chain_for(coll.and_then(|c| self.collection(c)), draft);
        self.seq += 1;
        let seq = self.seq;
        let (tx, ctx, settings, jar) = (
            self.tx.clone(),
            ctx.clone(),
            self.ws.settings.clone(),
            self.cookies.clone(),
        );
        if !chain.is_empty() {
            // Scripts run on the worker thread too: `pm.sendRequest` may wait on the network.
            let request = draft.clone();
            let auth = self.effective_auth(draft, origin);
            let scopes = self.scopes_for(coll);
            let info = crate::script::Info {
                request_name: draft.name.clone(),
                request_id: draft.id.to_string(),
                iteration: 1,
                iteration_count: 1,
            };
            self.pending.insert(tab_id, (seq, Instant::now()));
            std::thread::spawn(move || {
                let x = crate::script::exchange(request, &auth, &chain, scopes, &info, &settings, &jar);
                let _ = tx.send(Msg::Done {
                    tab: tab_id,
                    seq,
                    coll,
                    origin: origin_req,
                    request: Box::new(x.request),
                    sent: x.sent,
                    outcome: x.outcome,
                    report: Some(Box::new(x.report)),
                });
                ctx.request_repaint();
            });
            return;
        }
        let prepared = self.prepare(draft, origin);
        let request = Box::new(draft.clone());
        self.pending.insert(tab_id, (seq, Instant::now()));
        std::thread::spawn(move || {
            let (outcome, sent) = http::send(&prepared, &settings, &jar);
            let _ = tx.send(Msg::Done {
                tab: tab_id,
                seq,
                coll,
                origin: origin_req,
                request,
                sent,
                outcome,
                report: None,
            });
            ctx.request_repaint();
        });
    }

    fn finish(
        &mut self,
        tab: Id,
        seq: u64,
        coll: Option<Id>,
        origin: Option<Id>,
        request: Request,
        sent: Prepared,
        outcome: Outcome,
        report: Option<crate::script::Report>,
    ) {
        // Script variable changes count even if the tab was closed meanwhile.
        if let Some(r) = &report {
            self.apply_script_changes(coll, &r.changes);
        }
        // Record history and the console even if the tab was closed meanwhile.
        let time_ms = outcome.time_ms() as u64;
        self.ws.history.insert(
            0,
            HistoryEntry {
                id: new_id(),
                at: http::now_secs(),
                request: request.clone(),
                status: outcome.status(),
                time_ms,
            },
        );
        self.ws
            .history
            .truncate(self.ws.settings.history_limit.max(1));
        let logs = report.as_ref().map(script_log_lines).unwrap_or_default();
        self.push_console(sent.clone(), outcome.clone(), logs);
        if let Some(r) = origin {
            self.last_status.insert(r, outcome.status());
        }
        if self.pending.get(&tab).map(|p| p.0) != Some(seq) {
            return; // Cancelled, or sent again since.
        }
        self.pending.remove(&tab);
        let mut results = tests::run(&request.tests, &outcome);
        if let Some(r) = &report {
            results.extend(r.tests.iter().cloned());
        }
        let captured = tests::captures(&request.captures, &outcome);
        if !captured.is_empty() {
            self.apply_captures(coll, &captured);
        }
        let mut view = ResponseView::new(outcome, sent, results, captured);
        view.script = report;
        self.responses.insert(tab, view);
    }

    fn push_console(&mut self, sent: Prepared, outcome: Outcome, logs: Vec<crate::script::LogLine>) {
        self.console.push(ConsoleEntry {
            at: http::now_secs(),
            sent,
            outcome,
            logs,
        });
        if self.console.len() > 300 {
            self.console.remove(0);
        }
    }

    /// Copies of every variable scope, for scripts to read and change.
    pub fn scopes_for(&self, coll: Option<Id>) -> crate::script::Scopes {
        crate::script::Scopes {
            globals: self.ws.globals.clone(),
            collection: coll.and_then(|id| self.collection(id)).map(|c| c.variables.clone()).unwrap_or_default(),
            environment: self.active_env().map(|e| e.values.clone()).unwrap_or_default(),
            has_env: self.ws.active_env.is_some(),
            env_name: self.active_env().map(|e| e.name.clone()),
            vault: self.ws.vault.clone(),
            ..Default::default()
        }
    }

    /// The variable list a scope maps to (environment and collection fall back to globals).
    fn scope_list(&mut self, coll: Option<Id>, scope: Scope) -> Option<&mut Vec<KeyValue>> {
        match scope {
            Scope::Environment => match self.ws.active_env {
                Some(id) => self.ws.environments.iter_mut().find(|e| e.id == id).map(|e| &mut e.values),
                None => Some(&mut self.ws.globals),
            },
            Scope::Collection => match coll {
                Some(id) => self.ws.collections.iter_mut().find(|c| c.id == id).map(|c| &mut c.variables),
                None => Some(&mut self.ws.globals),
            },
            Scope::Global => Some(&mut self.ws.globals),
        }
    }

    /// Applies what scripts did with `pm.environment.set` and friends.
    pub fn apply_script_changes(&mut self, coll: Option<Id>, changes: &[crate::script::VarChange]) {
        use crate::script::VarChange;
        for c in changes {
            match c {
                VarChange::Set(scope, k, v) => {
                    if let Some(list) = self.scope_list(coll, *scope) {
                        collection::set_var(list, k, v);
                    }
                }
                VarChange::Unset(scope, k) => {
                    if let Some(list) = self.scope_list(coll, *scope) {
                        list.retain(|kv| &kv.key != k);
                    }
                }
                VarChange::Clear(scope) => {
                    if let Some(list) = self.scope_list(coll, *scope) {
                        list.clear();
                    }
                }
                VarChange::Vault(k, Some(v)) => collection::set_var(&mut self.ws.vault, k, v),
                VarChange::Vault(k, None) => self.ws.vault.retain(|kv| &kv.key != k),
            }
        }
    }

    /// Stores captured values. Environment captures go to globals when no environment is active.
    pub fn apply_captures(&mut self, coll: Option<Id>, captured: &[(Scope, String, String)]) {
        for (scope, key, value) in captured {
            let list = match scope {
                Scope::Environment => match self.ws.active_env {
                    Some(id) => self
                        .ws
                        .environments
                        .iter_mut()
                        .find(|e| e.id == id)
                        .map(|e| &mut e.values),
                    None => Some(&mut self.ws.globals),
                },
                Scope::Collection => match coll {
                    Some(id) => self
                        .ws
                        .collections
                        .iter_mut()
                        .find(|c| c.id == id)
                        .map(|c| &mut c.variables),
                    None => Some(&mut self.ws.globals),
                },
                Scope::Global => Some(&mut self.ws.globals),
            };
            if let Some(list) = list {
                collection::set_var(list, key, value);
            }
        }
    }

    fn fetch_token(&mut self, ctx: &egui::Context, tab_id: Id) {
        let Some(tab) = self.tab(tab_id) else { return };
        let TabKind::Request { draft, origin } = &tab.kind else {
            return;
        };
        let auth = self.effective_auth(draft, *origin);
        let vars = self.vars_for(origin.map(|o| o.coll));
        let (tx, ctx, settings) = (self.tx.clone(), ctx.clone(), self.ws.settings.clone());
        self.fetching_token.insert(tab_id);
        std::thread::spawn(move || {
            let result = http::fetch_oauth2_token(&auth, &vars, &settings);
            let _ = tx.send(Msg::Token {
                tab: tab_id,
                result,
            });
            ctx.request_repaint();
        });
    }

    pub fn socket_connect(&mut self, ctx: &egui::Context, tab_id: Id) {
        let Some(TabKind::Request { draft, origin }) = self.tab(tab_id).map(|t| &t.kind) else {
            return;
        };
        let socket_io = draft.kind == collection::RequestKind::SocketIo;
        // Same variables, headers and auth as an HTTP request would get.
        let prepared = self.prepare(draft, *origin);
        let headers: Vec<(String, String)> = prepared
            .headers
            .into_iter()
            .filter(|(k, _)| !k.eq_ignore_ascii_case("content-type"))
            .collect();
        let session = crate::ws::connect(&prepared.url, headers, socket_io, &self.ws.settings, ctx.clone());
        self.sockets.insert(
            tab_id,
            SocketView {
                session: Some(session),
                state: SocketState::Connecting,
                reason: String::new(),
                url: prepared.url,
                handshake: vec![],
                log: vec![],
                filter: String::new(),
            },
        );
    }

    fn socket_send(&mut self, tab_id: Id) {
        let Some(TabKind::Request { draft, origin }) = self.tab(tab_id).map(|t| &t.kind) else {
            return;
        };
        let vars = self.vars_for(origin.map(|o| o.coll));
        let text = http::resolve(&draft.ws_message, &vars);
        let event = (draft.kind == collection::RequestKind::SocketIo)
            .then(|| http::resolve(&draft.sio_event, &vars))
            .map(|e| if e.trim().is_empty() { "message".to_string() } else { e.trim().to_string() });
        if let Some(session) = self.sockets.get(&tab_id).and_then(|s| s.session.as_ref()) {
            let _ = session.commands.send(crate::ws::Command::Send { event, text });
        }
    }

    fn socket_disconnect(&mut self, tab_id: Id) {
        if let Some(session) = self.sockets.get(&tab_id).and_then(|s| s.session.as_ref()) {
            let _ = session.commands.send(crate::ws::Command::Close);
        }
    }

    /// Takes in what the socket threads reported.
    fn receive_sockets(&mut self) {
        for view in self.sockets.values_mut() {
            let Some(session) = &view.session else { continue };
            let mut done = false;
            while let Ok(event) = session.events.try_recv() {
                match event {
                    crate::ws::Event::Open { status, headers } => {
                        view.state = SocketState::Open;
                        view.handshake = headers;
                        view.handshake.insert(0, ("Status".into(), status));
                    }
                    crate::ws::Event::Message(m) => view.log.push(m),
                    crate::ws::Event::Closed(reason) | crate::ws::Event::Failed(reason) => {
                        view.state = SocketState::Closed;
                        view.reason = reason;
                        done = true;
                    }
                }
            }
            if done {
                view.session = None;
            }
        }
    }

    fn fetch_schema(&mut self, ctx: &egui::Context, tab_id: Id) {
        let Some(tab) = self.tab(tab_id) else { return };
        let TabKind::Request { draft, origin } = &tab.kind else {
            return;
        };
        // Same URL, headers and auth as the request, with the introspection query as the body.
        let mut probe = draft.clone();
        probe.method = "POST".into();
        probe.body_type = BodyType::GraphQl;
        probe.gql_query = crate::graphql::INTROSPECTION_QUERY.into();
        probe.gql_variables = "{}".into();
        let prepared = self.prepare(&probe, *origin);
        let (tx, ctx, settings, jar) = (self.tx.clone(), ctx.clone(), self.ws.settings.clone(), self.cookies.clone());
        self.fetching_schema.insert(tab_id);
        std::thread::spawn(move || {
            let (outcome, _) = http::send(&prepared, &settings, &jar);
            let result = match outcome {
                Outcome::Response { status, body, .. } => serde_json::from_str::<Value>(&body)
                    .map_err(|_| format!("The server answered {status}, but not with JSON. Is this a GraphQL endpoint?"))
                    .and_then(|json| crate::graphql::parse_introspection(&json)),
                Outcome::Error { message, .. } => Err(message),
            };
            let _ = tx.send(Msg::Schema { tab: tab_id, result });
            ctx.request_repaint();
        });
    }

    pub fn start_runner(&mut self, ctx: &egui::Context) {
        let Some(c) = self.runner.coll.and_then(|id| self.collection(id)).cloned() else {
            return;
        };
        let all: Vec<Request> = c
            .requests_in(self.runner.folder)
            .into_iter()
            .cloned()
            .collect();
        // Live connections stay open; there's nothing to "run once" about them.
        let requests: Vec<Request> = all.iter().filter(|r| r.kind == collection::RequestKind::Http).cloned().collect();
        let skipped = all.len() - requests.len();
        if skipped > 0 && !requests.is_empty() {
            self.notify(
                format!("Skipping {skipped} WebSocket / Socket.IO request{}: open them to connect", if skipped == 1 { "" } else { "s" }),
                ToastKind::Info,
            );
        }
        if requests.is_empty() {
            self.notify(
                "Nothing to run: this folder has no requests",
                ToastKind::Info,
            );
            return;
        }
        let data = self.runner.data.clone();
        let iterations = if data.is_empty() { self.runner.iterations.max(1) } else { data.len() as u32 };
        let stop = Arc::new(AtomicBool::new(false));
        self.runner.stop = stop.clone();
        self.runner.results.clear();
        self.runner.expanded.clear();
        self.runner.total = requests.len() * iterations as usize;
        self.runner.running = true;
        self.runner.started = Some(Instant::now());
        self.runner.elapsed = Duration::ZERO;

        let (tx, ctx, settings, jar) = (
            self.tx.clone(),
            ctx.clone(),
            self.ws.settings.clone(),
            self.cookies.clone(),
        );
        let mut globals = self.ws.globals.clone();
        let mut env = self
            .active_env()
            .map(|e| e.values.clone())
            .unwrap_or_default();
        let has_env = self.ws.active_env.is_some();
        let (delay, stop_on_fail, coll_id) = (self.runner.delay_ms, self.runner.stop_on_fail, c.id);
        let mut c = c;
        let env_name = self.active_env().map(|e| e.name.clone());
        let mut vault = self.ws.vault.clone();
        // One request after another, like Postman's runner, so captures feed later requests.
        std::thread::spawn(move || {
            'outer: for iteration in 1..=iterations {
                let mut i = 0;
                // `setNextRequest` can loop; don't let it loop forever.
                let mut steps = 0;
                while i < requests.len() && steps < requests.len() * 100 {
                    steps += 1;
                    let r = &requests[i];
                    if stop.load(Ordering::Relaxed) {
                        break 'outer;
                    }
                    let scopes = crate::script::Scopes {
                        globals: globals.clone(),
                        collection: c.variables.clone(),
                        environment: env.clone(),
                        has_env,
                        env_name: env_name.clone(),
                        // Data file columns override the environment, as in Postman.
                        data: data.get(iteration as usize - 1).cloned().unwrap_or_default().into_iter().collect(),
                        extra: vec![KeyValue::new("iteration", &iteration.to_string())],
                        vault: vault.clone(),
                        ..Default::default()
                    };
                    let chain = crate::script::chain_for(Some(&c), r);
                    let info = crate::script::Info {
                        request_name: r.name.clone(),
                        request_id: r.id.to_string(),
                        iteration,
                        iteration_count: iterations,
                    };
                    let x = crate::script::exchange(r.clone(), &c.effective_auth(r.id, &r.auth), &chain, scopes, &info, &settings, &jar);
                    // Scripts may have changed variables: later requests see that.
                    globals = x.scopes.globals;
                    c.variables = x.scopes.collection;
                    env = x.scopes.environment;
                    vault = x.scopes.vault;
                    let (outcome, sent, report) = (x.outcome, x.sent, x.report);
                    let mut tests = tests::run(&r.tests, &outcome);
                    tests.extend(report.tests.iter().cloned());
                    // A script that crashed is a failure, like a failed test.
                    tests.extend(report.errors.iter().map(|e| TestResult {
                        name: format!("{} script error", e.script),
                        passed: false,
                        detail: e.message.clone(),
                    }));
                    let captured = tests::captures(&r.captures, &outcome);
                    for (scope, k, v) in &captured {
                        let list = match scope {
                            Scope::Environment if has_env => &mut env,
                            Scope::Collection => &mut c.variables,
                            _ => &mut globals,
                        };
                        collection::set_var(list, k, v);
                    }
                    let (size, error) = match &outcome {
                        Outcome::Response { bytes, .. } => (bytes.len(), None),
                        Outcome::Error { message, .. } => (0, Some(message.clone())),
                    };
                    let result = RunResult {
                        iteration,
                        name: r.name.clone(),
                        method: r.method.clone(),
                        url: sent.url.clone(),
                        status: outcome.status(),
                        time_ms: outcome.time_ms(),
                        size,
                        tests,
                        error,
                    };
                    let failed = !result.passed();
                    let next = report.next_request.clone();
                    if tx
                        .send(Msg::RunResult(
                            result,
                            Some(coll_id),
                            captured,
                            Box::new((sent, outcome, report)),
                        ))
                        .is_err()
                    {
                        return;
                    }
                    ctx.request_repaint();
                    if failed && stop_on_fail {
                        break 'outer;
                    }
                    if delay > 0 {
                        std::thread::sleep(Duration::from_millis(delay));
                    }
                    // `pm.execution.setNextRequest(name)` jumps, `(null)` ends this iteration.
                    i = match next {
                        Some(Some(name)) => match requests.iter().position(|q| q.name == name) {
                            Some(j) => j,
                            None => i + 1,
                        },
                        Some(None) => requests.len(),
                        None => i + 1,
                    };
                }
            }
            let _ = tx.send(Msg::RunFinished);
            ctx.request_repaint();
        });
    }

    fn receive(&mut self) {
        self.receive_sockets();
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Msg::Done {
                    tab,
                    seq,
                    coll,
                    origin,
                    request,
                    sent,
                    outcome,
                    report,
                } => self.finish(tab, seq, coll, origin, *request, sent, outcome, report.map(|r| *r)),
                Msg::RunResult(result, coll, captured, exchange) => {
                    self.apply_captures(coll, &captured);
                    let (sent, outcome, report) = *exchange;
                    self.apply_script_changes(coll, &report.changes);
                    self.push_console(sent, outcome, script_log_lines(&report));
                    self.runner.results.push(result);
                }
                Msg::RunFinished => {
                    self.runner.running = false;
                    self.runner.elapsed =
                        self.runner.started.map(|s| s.elapsed()).unwrap_or_default();
                    let failed = self.runner.results.iter().filter(|r| !r.passed()).count();
                    if failed == 0 {
                        self.notify(
                            format!("Run finished: all {} passed", self.runner.results.len()),
                            ToastKind::Success,
                        );
                    } else {
                        self.notify(format!("Run finished: {failed} failed"), ToastKind::Error);
                    }
                }
                Msg::Schema { tab, result } => {
                    self.fetching_schema.remove(&tab);
                    match &result {
                        Ok(schema) => {
                            let msg = format!("Loaded the schema: {} types", schema.user_type_count());
                            self.notify(msg, ToastKind::Success);
                        }
                        Err(e) => {
                            let msg = format!("Couldn't load the schema: {e}");
                            self.notify(msg, ToastKind::Error);
                        }
                    }
                    self.gql_schemas.insert(tab, result);
                }
                Msg::Token { tab, result } => {
                    self.fetching_token.remove(&tab);
                    match result {
                        Ok(token) => {
                            if let Some(i) = self.tab_index(tab) {
                                if let TabKind::Request { draft, .. } = &mut self.ws.tabs[i].kind {
                                    if let Auth::OAuth2 { access_token, .. } = &mut draft.auth {
                                        *access_token = token;
                                    }
                                }
                            }
                            self.notify("Got a new access token", ToastKind::Success);
                        }
                        Err(e) => {
                            self.notify(format!("Token request failed: {e}"), ToastKind::Error)
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Import / export
    // -----------------------------------------------------------------------------------------

    pub fn import_dialog(&mut self) {
        self.dialog = Some(Dialog::Import {
            text: String::new(),
            error: None,
        });
    }

    pub fn pick_import_files(&mut self) {
        if let Some(paths) = rfd::FileDialog::new()
            .add_filter("Postman, OpenAPI, Insomnia, HAR", &["json", "yaml", "yml", "har"])
            .pick_files()
        {
            self.import_files(&paths);
        }
    }

    pub fn import_files(&mut self, paths: &[PathBuf]) {
        for path in paths {
            match std::fs::read_to_string(path) {
                Ok(text) => {
                    if let Err(e) = self.import_text(&text) {
                        self.notify(
                            format!(
                                "{}: {e}",
                                path.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default()
                            ),
                            ToastKind::Error,
                        );
                    }
                }
                Err(e) => self.notify(
                    format!("Could not open {}: {e}", path.display()),
                    ToastKind::Error,
                ),
            }
        }
    }

    pub fn import_text(&mut self, text: &str) -> Result<(), String> {
        let imported = import::detect(text)?;
        let what = imported.describe();
        match imported {
            Imported::Collection(c) => {
                let (cid, first) = (c.id, c.requests().first().map(|r| r.id));
                self.ws.open_nodes.insert(cid);
                self.ws.collections.push(c);
                self.ws.sidebar_view = SidebarView::Collections;
                if let Some(r) = first {
                    self.open_request(cid, r);
                }
            }
            Imported::Environment(e) => {
                self.ws.sidebar_view = SidebarView::Environments;
                if self.ws.active_env.is_none() {
                    self.ws.active_env = Some(e.id);
                }
                self.ws.environments.push(e);
            }
            Imported::Globals(g) => {
                for v in g {
                    collection::set_var(&mut self.ws.globals, &v.key, &v.value);
                }
            }
            Imported::Request(r) => {
                self.push_tab(Tab::new(TabKind::Request {
                    draft: r,
                    origin: None,
                }));
            }
            Imported::Workspace(colls, envs) => {
                for c in colls {
                    self.ws.open_nodes.insert(c.id);
                    self.ws.collections.push(c);
                }
                self.ws.environments.extend(envs);
            }
        }
        self.notify(format!("Imported {what}"), ToastKind::Success);
        Ok(())
    }

    fn save_file(&mut self, name: &str, content: &[u8], what: &str) {
        if let Some(path) = rfd::FileDialog::new().set_file_name(name).save_file() {
            match std::fs::write(&path, content) {
                Ok(()) => self.notify(
                    format!("Saved {what} to {}", path.display()),
                    ToastKind::Success,
                ),
                Err(e) => self.notify(format!("Could not save: {e}"), ToastKind::Error),
            }
        }
    }

    pub fn export_collection(&mut self, coll: Id, format: Format) {
        let Some(c) = self.collection(coll) else {
            return;
        };
        let content = crate::export::export(c, format, &self.vars_for(Some(coll)));
        let name = format!("{}.{}", slug(&c.name), format.extension());
        self.save_file(&name, content.as_bytes(), &format!("\"{}\"", c.name));
    }

    fn export_environment(&mut self, env: Id) {
        let Some(e) = self.ws.environments.iter().find(|e| e.id == env) else {
            return;
        };
        let content =
            serde_json::to_string_pretty(&collection::environment_to_json(e)).unwrap_or_default();
        let name = format!("{}.postman_environment.json", slug(&e.name));
        self.save_file(&name, content.as_bytes(), "the environment");
    }

    fn save_response(&mut self, tab: Id) {
        let Some(view) = self.responses.get(&tab) else {
            return;
        };
        let Outcome::Response { bytes, .. } = &view.outcome else {
            return;
        };
        let ext = match view.outcome.header("content-type").unwrap_or("") {
            ct if ct.contains("json") => "json",
            ct if ct.contains("html") => "html",
            ct if ct.contains("xml") => "xml",
            ct if ct.contains("png") => "png",
            ct if ct.contains("jpeg") => "jpg",
            ct if ct.contains("pdf") => "pdf",
            _ => "txt",
        };
        let bytes = bytes.clone();
        self.save_file(&format!("response.{ext}"), &bytes, "the response");
    }

    // -----------------------------------------------------------------------------------------
    // Actions
    // -----------------------------------------------------------------------------------------

    fn target_name(&self, t: Target) -> String {
        match t {
            Target::Collection(c) => self.collection(c).map(|c| c.name.clone()),
            Target::Node(c, n) => self
                .collection(c)
                .and_then(|c| c.find(n))
                .map(|n| n.name().to_string()),
            Target::Environment(e) => self
                .ws
                .environments
                .iter()
                .find(|x| x.id == e)
                .map(|e| e.name.clone()),
            Target::History(h) => self
                .ws
                .history
                .iter()
                .find(|x| x.id == h)
                .map(|h| h.request.name.clone()),
        }
        .unwrap_or_default()
    }

    pub fn rename(&mut self, t: Target, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        match t {
            Target::Collection(c) => {
                if let Some(c) = self.collection_mut(c) {
                    c.name = name;
                }
            }
            Target::Node(c, n) => {
                if let Some(node) = self.collection_mut(c).and_then(|c| c.find_mut(n)) {
                    *node.name_mut() = name.clone();
                }
                // Keep open tabs in step, without making them look edited.
                for tab in &mut self.ws.tabs {
                    if let TabKind::Request {
                        draft,
                        origin: Some(o),
                    } = &mut tab.kind
                    {
                        if o.req == n {
                            draft.name = name.clone();
                        }
                    }
                }
            }
            Target::Environment(e) => {
                if let Some(e) = self.ws.environments.iter_mut().find(|x| x.id == e) {
                    e.name = name;
                }
            }
            Target::History(_) => {}
        }
    }

    pub fn delete(&mut self, t: Target) {
        match t {
            Target::Collection(id) => {
                self.ws.collections.retain(|c| c.id != id);
                let gone: Vec<Id> = self
                    .ws
                    .tabs
                    .iter()
                    .filter(|t| match &t.kind {
                        TabKind::Collection { coll } | TabKind::Folder { coll, .. } => *coll == id,
                        _ => false,
                    })
                    .map(|t| t.id)
                    .collect();
                gone.into_iter().for_each(|t| self.close_tab(t));
                for tab in &mut self.ws.tabs {
                    if let TabKind::Request { origin, .. } = &mut tab.kind {
                        if origin.is_some_and(|o| o.coll == id) {
                            *origin = None;
                        }
                    }
                }
            }
            Target::Node(c, n) => {
                let removed = self.collection_mut(c).and_then(|c| c.remove(n));
                let gone: Vec<Id> = self
                    .ws
                    .tabs
                    .iter()
                    .filter(|t| match &t.kind {
                        TabKind::Request {
                            origin: Some(o), ..
                        } => removed.as_ref().is_some_and(|r| r.contains(o.req)),
                        TabKind::Folder { folder, .. } => {
                            removed.as_ref().is_some_and(|r| r.contains(*folder))
                        }
                        _ => false,
                    })
                    .map(|t| t.id)
                    .collect();
                gone.into_iter().for_each(|t| self.close_tab(t));
            }
            Target::Environment(id) => {
                self.ws.environments.retain(|e| e.id != id);
                if self.ws.active_env == Some(id) {
                    self.ws.active_env = None;
                }
                let gone: Vec<Id> = self
                    .ws
                    .tabs
                    .iter()
                    .filter(|t| matches!(t.kind, TabKind::Environment { env } if env == id))
                    .map(|t| t.id)
                    .collect();
                gone.into_iter().for_each(|t| self.close_tab(t));
            }
            Target::History(id) => self.ws.history.retain(|h| h.id != id),
        }
    }

    fn duplicate(&mut self, t: Target) {
        match t {
            Target::Collection(id) => {
                let Some(mut c) = self.collection(id).cloned() else {
                    return;
                };
                // Round-trip through the Postman format to get fresh ids everywhere.
                if let Ok(mut copy) = collection::parse(&collection::to_json(&c)) {
                    copy.name = format!("{} Copy", c.name);
                    c = copy;
                }
                let at = self
                    .ws
                    .collections
                    .iter()
                    .position(|x| x.id == id)
                    .map(|i| i + 1)
                    .unwrap_or(self.ws.collections.len());
                self.ws.collections.insert(at, c);
            }
            Target::Node(c, n) => {
                if let Some(new) = self.collection_mut(c).and_then(|c| c.duplicate(n)) {
                    if matches!(
                        self.collection(c).and_then(|x| x.find(new)),
                        Some(Node::Request(_))
                    ) {
                        self.open_request(c, new);
                    }
                }
            }
            Target::Environment(id) => {
                let Some(e) = self.ws.environments.iter().find(|e| e.id == id) else {
                    return;
                };
                let copy = Environment {
                    id: new_id(),
                    name: format!("{} Copy", e.name),
                    values: e.values.clone(),
                };
                self.ws.environments.push(copy);
            }
            Target::History(_) => {}
        }
    }

    fn move_node(&mut self, from: Id, node: Id, to: Id, drop: DropAt) {
        // Never drop a folder into itself.
        let into_self = match drop {
            DropAt::Into(Some(f)) | DropAt::Before(f) => self
                .collection(from)
                .and_then(|c| c.find(node))
                .is_some_and(|n| n.contains(f)),
            DropAt::Into(None) => false,
        };
        if into_self {
            return;
        }
        let Some(n) = self.collection_mut(from).and_then(|c| c.remove(node)) else {
            return;
        };
        let Some(target) = self.collection_mut(to) else {
            return;
        };
        match drop {
            DropAt::Into(folder) => match target.items_mut(folder) {
                Some(items) => items.push(n),
                None => target.items.push(n),
            },
            DropAt::Before(before) => target.insert_before(before, n),
        }
        if from != to {
            for tab in &mut self.ws.tabs {
                if let TabKind::Request {
                    origin: Some(o), ..
                } = &mut tab.kind
                {
                    if o.coll == from
                        && self
                            .ws
                            .collections
                            .iter()
                            .any(|c| c.id == to && c.find(o.req).is_some())
                    {
                        o.coll = to;
                    }
                }
            }
        }
        if let DropAt::Into(Some(f)) = drop {
            self.ws.open_nodes.insert(f);
        }
    }

    fn process_actions(&mut self, ctx: &egui::Context) {
        let actions = std::mem::take(&mut self.actions);
        for action in actions {
            match action {
                Action::Send(t) => self.send(ctx, t),
                Action::Cancel(t) => {
                    self.pending.remove(&t);
                }
                Action::Save(t) => self.save_tab(t, false),
                Action::CloseTab(t) => self.request_close(t),
                Action::ForceCloseTab(t) => self.close_tab(t),
                Action::CloseOtherTabs(keep) => {
                    let others: Vec<Id> = self
                        .ws
                        .tabs
                        .iter()
                        .filter(|t| t.id != keep)
                        .map(|t| t.id)
                        .collect();
                    for t in others {
                        if self.tab(t).is_some_and(|t| !self.is_dirty(t)) {
                            self.close_tab(t);
                        }
                    }
                }
                Action::FocusTab(t) => self.focus(t),
                Action::NewRequestTab => self.new_request_tab(),
                Action::NewTyped(k) => self.new_typed_tab(k),
                Action::DuplicateTab(t) => {
                    if let Some(TabKind::Request { draft, .. }) = self.tab(t).map(|t| &t.kind) {
                        let mut copy = draft.clone();
                        copy.id = new_id();
                        copy.name = format!("{} Copy", copy.name);
                        self.push_tab(Tab::new(TabKind::Request {
                            draft: copy,
                            origin: None,
                        }));
                    }
                }
                Action::OpenRequest { coll, req } => self.open_request(coll, req),
                Action::OpenTab(kind) => self.open_tab(kind),
                Action::OpenHistory(h) => {
                    if let Some(entry) = self.ws.history.iter().find(|x| x.id == h) {
                        let mut draft = entry.request.clone();
                        draft.id = new_id();
                        self.push_tab(Tab::new(TabKind::Request {
                            draft,
                            origin: None,
                        }));
                    }
                }
                Action::AddDemo => {
                    let demo = demo_workspace();
                    let mut c = demo.collections.into_iter().next().expect("the demo has a collection");
                    c.name = unique_name(&self.ws.collections, &c.name);
                    self.ws.open_nodes.insert(c.id);
                    self.ws.collections.push(c);
                    if !self.ws.environments.iter().any(|e| e.name == "Demo") {
                        self.ws.environments.extend(demo.environments);
                    }
                    self.ws.sidebar_view = SidebarView::Collections;
                    self.ws.show_sidebar = true;
                    self.notify("Added the demo collection", ToastKind::Success);
                }
                Action::NewCollection => {
                    self.dialog = Some(Dialog::Prompt {
                        title: "New collection".into(),
                        text: "New Collection".into(),
                        kind: PromptKind::NewCollection,
                    })
                }
                Action::NewEnvironment => {
                    self.dialog = Some(Dialog::Prompt {
                        title: "New environment".into(),
                        text: "New Environment".into(),
                        kind: PromptKind::NewEnvironment,
                    })
                }
                Action::AddRequest { coll, folder } => {
                    let r = Request::new("New Request");
                    let rid = r.id;
                    if let Some(c) = self.collection_mut(coll) {
                        match c.items_mut(folder) {
                            Some(items) => items.push(Node::Request(r)),
                            None => c.items.push(Node::Request(r)),
                        }
                    }
                    self.ws.open_nodes.insert(coll);
                    if let Some(f) = folder {
                        self.ws.open_nodes.insert(f);
                    }
                    self.open_request(coll, rid);
                }
                Action::AddFolder { coll, parent } => {
                    self.dialog = Some(Dialog::Prompt {
                        title: "New folder".into(),
                        text: "New Folder".into(),
                        kind: PromptKind::NewFolder { coll, parent },
                    })
                }
                Action::Rename(t) => {
                    let text = self.target_name(t);
                    self.dialog = Some(Dialog::Prompt {
                        title: "Rename".into(),
                        text,
                        kind: PromptKind::Rename(t),
                    });
                }
                Action::Delete(t) => {
                    let name = self.target_name(t);
                    let what = match t {
                        Target::Collection(_) => "the collection",
                        Target::Node(c, n) => match self.collection(c).and_then(|c| c.find(n)) {
                            Some(Node::Folder(_)) => "the folder",
                            _ => "the request",
                        },
                        Target::Environment(_) => "the environment",
                        Target::History(_) => "this history entry",
                    };
                    self.dialog = Some(Dialog::Confirm {
                        title: format!("Delete \"{name}\"?"),
                        message: format!(
                            "This deletes {what} and everything in it. It can't be undone."
                        ),
                        kind: ConfirmKind::Delete(t),
                    });
                }
                Action::Duplicate(t) => self.duplicate(t),
                Action::Shift { coll, node, delta } => {
                    if let Some(c) = self.collection_mut(coll) {
                        c.shift(node, delta);
                    }
                }
                Action::Move {
                    from,
                    node,
                    to,
                    drop,
                } => self.move_node(from, node, to, drop),
                Action::Export(c, f) => self.export_collection(c, f),
                Action::ExportEnvironment(e) => self.export_environment(e),
                Action::Import => self.import_dialog(),
                Action::ImportFiles(paths) => self.import_files(&paths),
                Action::Run { coll, folder } => {
                    self.runner.coll = Some(coll);
                    self.runner.folder = folder;
                    self.runner.open = true;
                    if !self.runner.running {
                        self.runner.results.clear();
                    }
                }
                Action::FetchToken(t) => self.fetch_token(ctx, t),
                Action::FetchSchema(t) => self.fetch_schema(ctx, t),
                Action::SocketConnect(t) => self.socket_connect(ctx, t),
                Action::SocketDisconnect(t) => self.socket_disconnect(t),
                Action::SocketSend(t) => self.socket_send(t),
                Action::SocketClear(t) => {
                    if let Some(v) = self.sockets.get_mut(&t) {
                        v.log.clear();
                    }
                }
                Action::Captures(coll, list) => self.apply_captures(coll, &list),
                Action::SetActiveEnv(e) => self.ws.active_env = e,
                Action::SaveResponse(t) => self.save_response(t),
                Action::Notify(text, kind) => self.notify(text, kind),
                Action::StartMock(c) => self.start_mock(ctx, c),
                Action::StopMock(c) => {
                    self.stop_mock(c);
                    self.notify("Mock server stopped", ToastKind::Info);
                }
                Action::Undo => self.undo(),
                Action::Redo => self.redo(),
                Action::MoveTab { tab, before } => {
                    if let (Some(from), true) = (self.tab_index(tab), tab != before) {
                        let t = self.ws.tabs.remove(from);
                        let to = self.tab_index(before).unwrap_or(self.ws.tabs.len());
                        self.ws.tabs.insert(to, t);
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------------------------------

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                self.dialog = None;
            }
            return;
        }
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;
        let cmd_alt = Modifiers::COMMAND | Modifiers::ALT;
        let active = self.ws.active_tab;
        let mut send = false;
        let typing = ctx.egui_wants_keyboard_input();
        ctx.input_mut(|i| {
            if i.consume_key(cmd, Key::Enter) {
                send = true;
            }
            if i.consume_key(cmd, Key::S) {
                if let Some(t) = active {
                    self.actions.push(Action::Save(t));
                }
            }
            if i.consume_key(cmd, Key::T) || i.consume_key(cmd, Key::N) {
                self.actions.push(Action::NewRequestTab);
            }
            if i.consume_key(cmd, Key::W) {
                if let Some(t) = active {
                    self.actions.push(Action::CloseTab(t));
                }
            }
            // Text fields keep their own undo; elsewhere it undoes workspace changes.
            if !typing && (i.consume_key(cmd_shift, Key::Z) || i.consume_key(cmd, Key::Y)) {
                self.actions.push(Action::Redo);
            }
            if !typing && i.consume_key(cmd, Key::Z) {
                self.actions.push(Action::Undo);
            }
            if i.consume_key(cmd, Key::O) {
                self.actions.push(Action::Import);
            }
            if i.consume_key(cmd, Key::K) {
                self.ws.show_sidebar = true;
                self.focus_search = true;
            }
            if i.consume_key(cmd, Key::B) {
                self.ws.show_sidebar = !self.ws.show_sidebar;
            }
            if i.consume_key(cmd, Key::Comma) {
                self.show_settings = !self.show_settings;
            }
            if self.ws.features.console && i.consume_key(cmd_alt, Key::C) {
                self.show_console = !self.show_console;
            }
            if self.ws.features.runner && i.consume_key(cmd_shift, Key::R) {
                if let Some(c) = self.ws.collections.first().map(|c| c.id) {
                    self.runner.coll = self.runner.coll.or(Some(c));
                }
                self.runner.open = !self.runner.open;
            }
            let next = i.consume_key(cmd_shift, Key::CloseBracket)
                || i.consume_key(Modifiers::CTRL, Key::Tab);
            let prev = i.consume_key(cmd_shift, Key::OpenBracket)
                || i.consume_key(Modifiers::CTRL | Modifiers::SHIFT, Key::Tab);
            if (next || prev) && !self.ws.tabs.is_empty() {
                let n = self.ws.tabs.len();
                let cur = active
                    .and_then(|a| self.ws.tabs.iter().position(|t| t.id == a))
                    .unwrap_or(0);
                let to = if next {
                    (cur + 1) % n
                } else {
                    (cur + n - 1) % n
                };
                self.ws.active_tab = Some(self.ws.tabs[to].id);
            }
        });
        if send {
            if let Some(t) = active.filter(|t| !self.pending.contains_key(t)) {
                if matches!(self.tab(t).map(|t| &t.kind), Some(TabKind::Request { .. })) {
                    self.actions.push(Action::Send(t));
                }
            }
        }
    }

    fn toast_ui(&mut self, ctx: &egui::Context) {
        let Some(t) = &self.toast else { return };
        let life = Duration::from_millis(if t.kind == ToastKind::Error || t.undo {
            5200
        } else {
            2800
        });
        let left = life.saturating_sub(t.at.elapsed());
        if left.is_zero() {
            self.toast = None;
            return;
        }
        let p = theme::palette(ctx.global_style().visuals.dark_mode);
        let (icon, color) = match t.kind {
            ToastKind::Info => ("\u{E2CE}", p.blue),
            ToastKind::Success => ("\u{E182}", p.accent),
            ToastKind::Error => ("\u{E4E0}", p.red),
        };
        let text = t.text.clone();
        let with_undo = t.undo;
        let mut undo_clicked = false;
        egui::Area::new(egui::Id::unique("toast"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-20.0, -40.0))
            .order(egui::Order::Tooltip)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(p.surface)
                    .stroke(egui::Stroke::new(1.0, p.border))
                    .corner_radius(10)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .shadow(egui::epaint::Shadow {
                        offset: [0, 8],
                        blur: 24,
                        spread: 0,
                        color: Color32::from_black_alpha(90),
                    })
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new(icon).color(color).size(15.0));
                            ui.add(
                                egui::Label::new(egui::RichText::new(text).color(p.text)).wrap(),
                            );
                            if with_undo && ui.link(egui::RichText::new("Undo").strong().color(p.accent)).clicked() {
                                undo_clicked = true;
                            }
                        });
                    });
            });
        ctx.request_repaint_after(left.min(Duration::from_millis(250)));
        if undo_clicked {
            self.toast = None;
            self.actions.push(Action::Undo);
        }
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        self.tab_bar(ui);
        let Some(i) = self.ws.active_tab.and_then(|t| self.tab_index(t)) else {
            self.welcome(ui);
            return;
        };
        // Take the tab out while drawing it, so the editor can use the rest of `self` freely.
        let mut tab = std::mem::replace(&mut self.ws.tabs[i], Tab::new(TabKind::Globals));
        match &tab.kind {
            TabKind::Request { .. } => self.request_tab(ui, &mut tab),
            TabKind::Collection { .. } | TabKind::Folder { .. } => {
                self.collection_tab(ui, &mut tab)
            }
            TabKind::Environment { .. } | TabKind::Globals => self.environment_tab(ui, &mut tab),
        }
        self.ws.tabs[i] = tab;
    }
}

pub fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "collection".into() } else { s }
}

impl eframe::App for GazaMan {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.receive();
        self.lens.handle_input(&ctx);
        self.shortcuts(&ctx);
        if self.ws.dark != ctx.global_style().visuals.dark_mode {
            theme::set_dark(&ctx, self.ws.dark);
        }

        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            self.actions.push(Action::ImportFiles(dropped));
        }

        self.top_bar(ui);
        self.status_bar(ui);
        if self.show_console {
            self.console_panel(ui);
        }
        self.rail(ui);
        if self.ws.show_sidebar {
            self.sidebar(ui);
        }
        let p = theme::pal(ui);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.surface))
            .show(ui, |ui| self.central(ui));

        self.runner_window(&ctx);
        self.settings_window(&ctx);
        self.cookies_window(&ctx);
        self.dialogs(&ctx);
        self.toast_ui(&ctx);
        self.process_actions(&ctx);
        self.track_changes(&ctx);
        self.refresh_mocks();

        if !self.pending.is_empty() || self.runner.running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        let hovering_file = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering_file {
            let rect = ctx.content_rect();
            let painter = ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique("drop"),
            ));
            painter.rect_filled(rect, 0.0, Color32::from_black_alpha(170));
            painter.rect_stroke(
                rect.shrink(16.0),
                16.0,
                egui::Stroke::new(2.0, theme::FLAG_GREEN),
                egui::StrokeKind::Inside,
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "Drop to import",
                FontId::proportional(26.0),
                Color32::WHITE,
            );
            painter.text(
                rect.center() + egui::vec2(0.0, 32.0),
                egui::Align2::CENTER_CENTER,
                "Postman collections & environments · OpenAPI · Insomnia",
                FontId::proportional(14.0),
                Color32::from_gray(200),
            );
        }

        // Last, so the lens sees everything drawn this frame.
        self.lens.paint(&ctx, theme::FLAG_GREEN);

        if let Some(shot) = &mut self.shot {
            let busy = !self.pending.is_empty()
                || self.runner.running
                || !self.fetching_schema.is_empty()
                || self.sockets.values().any(|s| s.state == SocketState::Connecting);
            if let Some(step) = shot.tick(&ctx, busy) {
                crate::shot::apply(self, &ctx, &step);
            }
        }
    }

    /// Screenshot runs shouldn't change the saved panel sizes either.
    fn persist_egui_memory(&self) -> bool {
        self.shot.is_none()
    }

    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(events) = self.shot.as_mut().and_then(|s| s.input.pop_front()) {
            raw_input.events.extend(events);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if self.shot.is_some() {
            return;
        }
        if let Ok(jar) = self.cookies.lock() {
            self.ws.cookies = jar.clone();
        }
        if let Ok(s) = serde_json::to_string(&self.ws) {
            storage.set_string(STORAGE_KEY, s);
        }
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;

    #[test]
    fn workspace_round_trips_through_json() {
        let ws = demo_workspace();
        let json = serde_json::to_string(&ws).unwrap();
        let back: Workspace = serde_json::from_str(&json).unwrap();
        assert_eq!(back.collections, ws.collections);
        assert_eq!(back.collections[0].count_requests(), 19);
        assert_eq!(back.features, ws.features);
    }

    #[test]
    fn replaces_only_the_old_demo() {
        let mut ws = demo_workspace();
        let mut old = Collection::new("Gaza Man Demo");
        old.variables.push(KeyValue::new("baseUrl", "https://jsonplaceholder.typicode.com"));
        let mine = Collection::new("My API");
        ws.collections = vec![mine.clone(), old];
        replace_old_demo(&mut ws);
        assert_eq!(ws.collections[0], mine, "other collections stay");
        assert!(ws.collections[1].variables.iter().any(|v| v.value == "https://dummyjson.com"));
        // A demo the user changed (new baseUrl) is theirs now: leave it.
        let mut edited = ws.clone();
        edited.collections[1].variables[0].value = "http://localhost:3000".into();
        let before = edited.collections.clone();
        replace_old_demo(&mut edited);
        assert_eq!(edited.collections, before);
    }
}

/// A script report as console lines: what was logged, then the errors.
pub fn script_log_lines(r: &crate::script::Report) -> Vec<crate::script::LogLine> {
    let mut lines = r.logs.clone();
    lines.extend(r.errors.iter().map(|e| crate::script::LogLine {
        phase: e.phase,
        level: "error".into(),
        text: e.message.clone(),
        script: e.script.clone(),
    }));
    lines
}

/// Version 2.0's demo used jsonplaceholder.typicode.com, which some networks block. Swap it for
/// the current demo, keeping its place in the list. Edited copies are left alone.
fn replace_old_demo(ws: &mut Workspace) {
    let is_old = |c: &Collection| {
        c.name == "Gaza Man Demo" && c.variables.iter().any(|v| v.key == "baseUrl" && v.value.contains("jsonplaceholder.typicode.com"))
    };
    let Some(i) = ws.collections.iter().position(is_old) else { return };
    let demo = demo_workspace();
    let new = demo.collections.into_iter().next().expect("the demo has a collection");
    ws.open_nodes.insert(new.id);
    ws.collections[i] = new;
    if let Some(env) = ws.environments.iter_mut().find(|e| e.name == "Demo") {
        if !env.values.iter().any(|v| v.key == "token") {
            env.values.push(KeyValue::new("token", ""));
        }
    }
    // Tabs still pointing at the old requests would show "deleted"; close them.
    let ids: HashSet<Id> = ws.collections.iter().map(|c| c.id).collect();
    ws.tabs.retain(|t| match &t.kind {
        TabKind::Request { origin: Some(o), .. } => ids.contains(&o.coll),
        TabKind::Collection { coll } | TabKind::Folder { coll, .. } => ids.contains(coll),
        _ => true,
    });
    if ws.active_tab.is_some_and(|a| !ws.tabs.iter().any(|t| t.id == a)) {
        ws.active_tab = ws.tabs.first().map(|t| t.id);
    }
}

/// `name`, or `name 2`, `name 3`… if it's taken.
fn unique_name(collections: &[Collection], name: &str) -> String {
    if !collections.iter().any(|c| c.name == name) {
        return name.to_string();
    }
    (2..).map(|n| format!("{name} {n}")).find(|n| !collections.iter().any(|c| &c.name == n)).unwrap_or_default()
}
