//! The workspace model: collections (folders and requests), environments and history,
//! plus reading and writing the Postman v2 / v2.1 collection and environment formats.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub type Id = u64;

/// A random id. Ids only need to be unique inside one workspace.
pub fn new_id() -> Id {
    // Keep ids within JavaScript's safe integer range so exported files stay readable everywhere.
    crate::http::random_u64() & ((1 << 53) - 1)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    pub enabled: bool,
    /// Only used by form-data fields: `value` is then a file path.
    pub file: bool,
}

impl Default for KeyValue {
    fn default() -> Self {
        Self {
            key: String::new(),
            value: String::new(),
            enabled: true,
            file: false,
        }
    }
}

impl KeyValue {
    pub fn new(key: &str, value: &str) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            ..Default::default()
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BodyType {
    #[default]
    None,
    FormData,
    UrlEncoded,
    Raw,
    Binary,
    GraphQl,
}

impl BodyType {
    pub const ALL: [BodyType; 6] = [
        BodyType::None,
        BodyType::FormData,
        BodyType::UrlEncoded,
        BodyType::Raw,
        BodyType::Binary,
        BodyType::GraphQl,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BodyType::None => "none",
            BodyType::FormData => "form-data",
            BodyType::UrlEncoded => "x-www-form-urlencoded",
            BodyType::Raw => "raw",
            BodyType::Binary => "binary",
            BodyType::GraphQl => "GraphQL",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawLang {
    #[default]
    Json,
    Text,
    JavaScript,
    Html,
    Xml,
}

impl RawLang {
    pub const ALL: [RawLang; 5] = [
        RawLang::Json,
        RawLang::Text,
        RawLang::JavaScript,
        RawLang::Html,
        RawLang::Xml,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RawLang::Json => "JSON",
            RawLang::Text => "Text",
            RawLang::JavaScript => "JavaScript",
            RawLang::Html => "HTML",
            RawLang::Xml => "XML",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            RawLang::Json => "application/json",
            RawLang::Text => "text/plain",
            RawLang::JavaScript => "application/javascript",
            RawLang::Html => "text/html",
            RawLang::Xml => "application/xml",
        }
    }

    fn postman(self) -> &'static str {
        match self {
            RawLang::Json => "json",
            RawLang::Text => "text",
            RawLang::JavaScript => "javascript",
            RawLang::Html => "html",
            RawLang::Xml => "xml",
        }
    }

    fn from_postman(s: &str) -> Self {
        match s {
            "text" => RawLang::Text,
            "javascript" => RawLang::JavaScript,
            "html" => RawLang::Html,
            "xml" => RawLang::Xml,
            _ => RawLang::Json,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Auth {
    #[default]
    Inherit,
    None,
    Bearer {
        token: String,
    },
    Basic {
        username: String,
        password: String,
    },
    ApiKey {
        key: String,
        value: String,
        in_query: bool,
    },
    OAuth2 {
        access_token: String,
        header_prefix: String,
        token_url: String,
        client_id: String,
        client_secret: String,
        scope: String,
        /// `false`: client credentials grant, `true`: password grant.
        password_grant: bool,
        username: String,
        password: String,
    },
    /// HTTP Digest: answered after the server's 401 challenge.
    Digest {
        username: String,
        password: String,
    },
    /// AWS Signature Version 4.
    Aws {
        access_key: String,
        secret_key: String,
        region: String,
        service: String,
        session_token: String,
    },
    /// Windows NTLM (v2), a challenge/response handshake on one connection.
    Ntlm {
        username: String,
        password: String,
        domain: String,
        workstation: String,
    },
}

impl Auth {
    pub fn label(&self) -> &'static str {
        match self {
            Auth::Inherit => "Inherit auth from parent",
            Auth::None => "No Auth",
            Auth::Bearer { .. } => "Bearer Token",
            Auth::Basic { .. } => "Basic Auth",
            Auth::ApiKey { .. } => "API Key",
            Auth::OAuth2 { .. } => "OAuth 2.0",
            Auth::Digest { .. } => "Digest Auth",
            Auth::Aws { .. } => "AWS Signature",
            Auth::Ntlm { .. } => "NTLM Authentication",
        }
    }

    pub fn choices() -> [Auth; 9] {
        [
            Auth::Inherit,
            Auth::None,
            Auth::Bearer {
                token: String::new(),
            },
            Auth::Basic {
                username: String::new(),
                password: String::new(),
            },
            Auth::ApiKey {
                key: String::new(),
                value: String::new(),
                in_query: false,
            },
            Auth::OAuth2 {
                access_token: String::new(),
                header_prefix: "Bearer".into(),
                token_url: String::new(),
                client_id: String::new(),
                client_secret: String::new(),
                scope: String::new(),
                password_grant: false,
                username: String::new(),
                password: String::new(),
            },
            Auth::Digest {
                username: String::new(),
                password: String::new(),
            },
            Auth::Aws {
                access_key: String::new(),
                secret_key: String::new(),
                region: "us-east-1".into(),
                service: String::new(),
                session_token: String::new(),
            },
            Auth::Ntlm {
                username: String::new(),
                password: String::new(),
                domain: String::new(),
                workstation: String::new(),
            },
        ]
    }
}

/// Where an assertion or a capture reads its value from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Source {
    #[default]
    Status,
    Time,
    Json,
    Header,
    Body,
}

impl Source {
    pub const ALL: [Source; 5] = [
        Source::Status,
        Source::Time,
        Source::Json,
        Source::Header,
        Source::Body,
    ];
    pub const CAPTURABLE: [Source; 4] =
        [Source::Json, Source::Header, Source::Status, Source::Body];

    pub fn label(self) -> &'static str {
        match self {
            Source::Status => "Status code",
            Source::Time => "Response time (ms)",
            Source::Json => "JSON body",
            Source::Header => "Header",
            Source::Body => "Body text",
        }
    }

    pub fn needs_path(self) -> bool {
        matches!(self, Source::Json | Source::Header)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    #[default]
    Equals,
    NotEquals,
    GreaterThan,
    LessThan,
    Contains,
    NotContains,
    Exists,
    NotExists,
}

impl Op {
    pub const ALL: [Op; 8] = [
        Op::Equals,
        Op::NotEquals,
        Op::GreaterThan,
        Op::LessThan,
        Op::Contains,
        Op::NotContains,
        Op::Exists,
        Op::NotExists,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Op::Equals => "equals",
            Op::NotEquals => "not equals",
            Op::GreaterThan => "greater than",
            Op::LessThan => "less than",
            Op::Contains => "contains",
            Op::NotContains => "not contains",
            Op::Exists => "exists",
            Op::NotExists => "does not exist",
        }
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, Op::Exists | Op::NotExists)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Assertion {
    pub enabled: bool,
    pub source: Source,
    pub path: String,
    pub op: Op,
    pub expected: String,
}

impl Default for Assertion {
    fn default() -> Self {
        Self {
            enabled: true,
            source: Source::Status,
            path: String::new(),
            op: Op::Equals,
            expected: "200".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    #[default]
    Environment,
    Collection,
    Global,
}

impl Scope {
    pub const ALL: [Scope; 3] = [Scope::Environment, Scope::Collection, Scope::Global];

    pub fn label(self) -> &'static str {
        match self {
            Scope::Environment => "Environment",
            Scope::Collection => "Collection",
            Scope::Global => "Globals",
        }
    }
}

/// Saves part of a response into a variable, so later requests can use it (e.g. a login token).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    pub enabled: bool,
    pub var: String,
    pub source: Source,
    pub path: String,
    pub scope: Scope,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            enabled: true,
            var: String::new(),
            source: Source::Json,
            path: String::new(),
            scope: Scope::Environment,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Request {
    pub id: Id,
    pub name: String,
    pub method: String,
    pub url: String,
    /// Query params, kept in sync with the URL. Disabled ones only live here.
    pub params: Vec<KeyValue>,
    /// Values for `:name` segments in the URL path.
    pub path_vars: Vec<KeyValue>,
    pub headers: Vec<KeyValue>,
    pub body_type: BodyType,
    pub raw_lang: RawLang,
    pub raw: String,
    pub form: Vec<KeyValue>,
    pub urlencoded: Vec<KeyValue>,
    pub binary_path: String,
    pub gql_query: String,
    pub gql_variables: String,
    pub auth: Auth,
    pub description: String,
    pub tests: Vec<Assertion>,
    pub captures: Vec<Capture>,
    /// Postman scripts (pre-request and test), in Postman's own `event` format.
    pub events: Vec<Value>,
    /// Saved responses: shown in docs and served by the mock server.
    pub examples: Vec<Example>,
    pub kind: RequestKind,
    /// The message being written, for WebSocket and Socket.IO requests.
    pub ws_message: String,
    /// The Socket.IO event to emit.
    pub sio_event: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestKind {
    #[default]
    Http,
    WebSocket,
    SocketIo,
}

/// A saved response for a request (Postman calls them examples).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Example {
    pub id: Id,
    pub name: String,
    pub status: u16,
    pub headers: Vec<KeyValue>,
    pub body: String,
}

impl Default for Example {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "Example".into(),
            status: 200,
            headers: vec![KeyValue::new("Content-Type", "application/json")],
            body: String::new(),
        }
    }
}

impl Example {
    pub fn content_type(&self) -> &str {
        self.headers
            .iter()
            .find(|h| h.enabled && h.key.eq_ignore_ascii_case("content-type"))
            .map(|h| h.value.as_str())
            .unwrap_or("text/plain")
    }
}

impl Default for Request {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "Untitled Request".into(),
            method: "GET".into(),
            url: String::new(),
            params: vec![],
            path_vars: vec![],
            headers: vec![],
            body_type: BodyType::None,
            raw_lang: RawLang::Json,
            raw: String::new(),
            form: vec![],
            urlencoded: vec![],
            binary_path: String::new(),
            gql_query: String::new(),
            gql_variables: String::new(),
            auth: Auth::Inherit,
            description: String::new(),
            tests: vec![],
            captures: vec![],
            events: vec![],
            examples: vec![],
            kind: RequestKind::Http,
            ws_message: String::new(),
            sio_event: "message".into(),
        }
    }
}

/// Splits `a=1&b=2` into key/value pairs (no percent-decoding, so `{{vars}}` survive).
pub fn parse_query(query: &str) -> Vec<KeyValue> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => KeyValue::new(k, v),
            None => KeyValue::new(p, ""),
        })
        .collect()
}

/// The URL without its `?query` and `#fragment`.
pub fn url_base(url: &str) -> &str {
    let end = url.find(['?', '#']).unwrap_or(url.len());
    &url[..end]
}

fn url_query(url: &str) -> &str {
    let Some(q) = url.find('?') else { return "" };
    let rest = &url[q + 1..];
    &rest[..rest.find('#').unwrap_or(rest.len())]
}

/// The `:name` path variables in a URL, in order.
pub fn path_var_names(url: &str) -> Vec<String> {
    let base = url_base(url);
    // Skip the scheme so `http://` isn't read as a variable.
    let path = base.split_once("://").map(|(_, rest)| rest).unwrap_or(base);
    path.split('/')
        .skip(1)
        .filter_map(|seg| seg.strip_prefix(':'))
        .map(|name| {
            name.trim_end_matches(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-'))
                .to_string()
        })
        .filter(|n| !n.is_empty())
        .collect()
}

impl Request {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    /// Call after editing the URL text: re-reads the query params and path variables.
    pub fn sync_from_url(&mut self) {
        let disabled: Vec<KeyValue> = self.params.iter().filter(|p| !p.enabled).cloned().collect();
        self.params = parse_query(url_query(&self.url));
        self.params.extend(disabled);
        self.sync_path_vars();
    }

    /// Call after editing the params table: rebuilds the URL's query string.
    pub fn sync_to_url(&mut self) {
        let fragment = self
            .url
            .find('#')
            .map(|i| self.url[i..].to_string())
            .unwrap_or_default();
        let mut url = url_base(&self.url).to_string();
        let query: Vec<String> = self
            .params
            .iter()
            .filter(|p| p.enabled && !(p.key.is_empty() && p.value.is_empty()))
            .map(|p| {
                if p.value.is_empty() && !p.key.is_empty() {
                    p.key.clone()
                } else {
                    format!("{}={}", p.key, p.value)
                }
            })
            .collect();
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query.join("&"));
        }
        url.push_str(&fragment);
        self.url = url;
    }

    pub fn sync_path_vars(&mut self) {
        let names = path_var_names(&self.url);
        let old = std::mem::take(&mut self.path_vars);
        self.path_vars = names
            .iter()
            .map(|n| {
                old.iter()
                    .find(|v| &v.key == n)
                    .cloned()
                    .unwrap_or_else(|| KeyValue::new(n, ""))
            })
            .collect();
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Folder {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub auth: Auth,
    pub items: Vec<Node>,
    pub events: Vec<Value>,
}

impl Default for Folder {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Folder".into(),
            description: String::new(),
            auth: Auth::Inherit,
            items: vec![],
            events: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Folder(Folder),
    Request(Request),
}

impl Node {
    pub fn id(&self) -> Id {
        match self {
            Node::Folder(f) => f.id,
            Node::Request(r) => r.id,
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Node::Folder(f) => &f.name,
            Node::Request(r) => &r.name,
        }
    }

    pub fn name_mut(&mut self) -> &mut String {
        match self {
            Node::Folder(f) => &mut f.name,
            Node::Request(r) => &mut r.name,
        }
    }

    /// True if `id` is this node or anything inside it.
    pub fn contains(&self, id: Id) -> bool {
        self.id() == id || matches!(self, Node::Folder(f) if f.items.iter().any(|n| n.contains(id)))
    }

    fn renew_ids(&mut self) {
        match self {
            Node::Request(r) => r.id = new_id(),
            Node::Folder(f) => {
                f.id = new_id();
                f.items.iter_mut().for_each(Node::renew_ids);
            }
        }
    }

    fn collect_requests<'a>(&'a self, out: &mut Vec<&'a Request>) {
        match self {
            Node::Request(r) => out.push(r),
            Node::Folder(f) => f.items.iter().for_each(|n| n.collect_requests(out)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Collection {
    pub id: Id,
    pub name: String,
    pub description: String,
    pub auth: Auth,
    pub variables: Vec<KeyValue>,
    pub items: Vec<Node>,
    pub events: Vec<Value>,
}

impl Default for Collection {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Collection".into(),
            description: String::new(),
            auth: Auth::None,
            variables: vec![],
            items: vec![],
            events: vec![],
        }
    }
}

fn find_in(items: &[Node], id: Id) -> Option<&Node> {
    items.iter().find_map(|n| {
        if n.id() == id {
            Some(n)
        } else if let Node::Folder(f) = n {
            find_in(&f.items, id)
        } else {
            None
        }
    })
}

fn find_in_mut(items: &mut [Node], id: Id) -> Option<&mut Node> {
    for n in items.iter_mut() {
        if n.id() == id {
            return Some(n);
        }
        if let Node::Folder(f) = n {
            if let Some(found) = find_in_mut(&mut f.items, id) {
                return Some(found);
            }
        }
    }
    None
}

/// The list that holds `id`, and its index there.
fn parent_list_mut(items: &mut Vec<Node>, id: Id) -> Option<(&mut Vec<Node>, usize)> {
    if let Some(i) = items.iter().position(|n| n.id() == id) {
        return Some((items, i));
    }
    for n in items.iter_mut() {
        if let Node::Folder(f) = n {
            if let Some(found) = parent_list_mut(&mut f.items, id) {
                return Some(found);
            }
        }
    }
    None
}

fn folders_on_path<'a>(items: &'a [Node], id: Id, path: &mut Vec<&'a Folder>) -> bool {
    for n in items {
        if n.id() == id {
            return true;
        }
        if let Node::Folder(f) = n {
            path.push(f);
            if folders_on_path(&f.items, id, path) {
                return true;
            }
            path.pop();
        }
    }
    false
}

fn list_folders(items: &[Node], prefix: &str, out: &mut Vec<(Id, String)>) {
    for n in items {
        if let Node::Folder(f) = n {
            let label = if prefix.is_empty() {
                f.name.clone()
            } else {
                format!("{prefix} / {}", f.name)
            };
            out.push((f.id, label.clone()));
            list_folders(&f.items, &label, out);
        }
    }
}

impl Collection {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            ..Default::default()
        }
    }

    pub fn find(&self, id: Id) -> Option<&Node> {
        find_in(&self.items, id)
    }

    pub fn find_mut(&mut self, id: Id) -> Option<&mut Node> {
        find_in_mut(&mut self.items, id)
    }

    pub fn request(&self, id: Id) -> Option<&Request> {
        match self.find(id) {
            Some(Node::Request(r)) => Some(r),
            _ => None,
        }
    }

    pub fn request_mut(&mut self, id: Id) -> Option<&mut Request> {
        match self.find_mut(id) {
            Some(Node::Request(r)) => Some(r),
            _ => None,
        }
    }

    pub fn folder_mut(&mut self, id: Id) -> Option<&mut Folder> {
        match self.find_mut(id) {
            Some(Node::Folder(f)) => Some(f),
            _ => None,
        }
    }

    /// The folders from the collection root down to `id` (not including `id` itself).
    pub fn path_to(&self, id: Id) -> Vec<&Folder> {
        let mut path = Vec::new();
        if folders_on_path(&self.items, id, &mut path) {
            path
        } else {
            Vec::new()
        }
    }

    /// The auth a request (or folder) with this `auth` actually uses, walking up its parents.
    pub fn effective_auth(&self, id: Id, own: &Auth) -> Auth {
        if *own != Auth::Inherit {
            return own.clone();
        }
        self.path_to(id)
            .iter()
            .rev()
            .map(|f| &f.auth)
            .find(|a| **a != Auth::Inherit)
            .unwrap_or(&self.auth)
            .clone()
    }

    /// Every folder as `(id, "Parent / Child")`, for pickers.
    pub fn folders(&self) -> Vec<(Id, String)> {
        let mut out = Vec::new();
        list_folders(&self.items, "", &mut out);
        out
    }

    pub fn requests(&self) -> Vec<&Request> {
        let mut out = Vec::new();
        self.items.iter().for_each(|n| n.collect_requests(&mut out));
        out
    }

    /// All requests inside a folder (or the whole collection), in order.
    pub fn requests_in(&self, folder: Option<Id>) -> Vec<&Request> {
        match folder.and_then(|id| self.find(id)) {
            Some(node) => {
                let mut out = Vec::new();
                node.collect_requests(&mut out);
                out
            }
            None => self.requests(),
        }
    }

    pub fn count_requests(&self) -> usize {
        self.requests().len()
    }

    /// The item list of a folder, or of the collection root for `None`.
    pub fn items_mut(&mut self, folder: Option<Id>) -> Option<&mut Vec<Node>> {
        match folder {
            None => Some(&mut self.items),
            Some(id) => self.folder_mut(id).map(|f| &mut f.items),
        }
    }

    pub fn remove(&mut self, id: Id) -> Option<Node> {
        let (list, i) = parent_list_mut(&mut self.items, id)?;
        Some(list.remove(i))
    }

    /// Inserts a copy (with new ids) right after the original. Returns the copy's id.
    pub fn duplicate(&mut self, id: Id) -> Option<Id> {
        let (list, i) = parent_list_mut(&mut self.items, id)?;
        let mut copy = list[i].clone();
        copy.renew_ids();
        let name = format!("{} Copy", copy.name());
        *copy.name_mut() = name;
        let new = copy.id();
        list.insert(i + 1, copy);
        Some(new)
    }

    /// Moves an item up (`-1`) or down (`+1`) among its siblings.
    pub fn shift(&mut self, id: Id, delta: isize) {
        if let Some((list, i)) = parent_list_mut(&mut self.items, id) {
            let j = i as isize + delta;
            if j >= 0 && (j as usize) < list.len() {
                list.swap(i, j as usize);
            }
        }
    }

    /// Puts `node` right before the item `before` (anywhere in the tree), or at the end.
    pub fn insert_before(&mut self, before: Id, node: Node) {
        match parent_list_mut(&mut self.items, before) {
            Some((list, i)) => list.insert(i, node),
            None => self.items.push(node),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Environment {
    pub id: Id,
    pub name: String,
    pub values: Vec<KeyValue>,
}

impl Default for Environment {
    fn default() -> Self {
        Self {
            id: new_id(),
            name: "New Environment".into(),
            values: vec![],
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryEntry {
    pub id: Id,
    /// Unix seconds.
    pub at: u64,
    pub request: Request,
    pub status: Option<u16>,
    pub time_ms: u64,
}

impl Default for HistoryEntry {
    fn default() -> Self {
        Self {
            id: new_id(),
            at: 0,
            request: Request::default(),
            status: None,
            time_ms: 0,
        }
    }
}

/// Sets `key` in a variable list, adding it if it's not there.
pub fn set_var(list: &mut Vec<KeyValue>, key: &str, value: &str) {
    match list.iter_mut().find(|v| v.key == key) {
        Some(v) => {
            v.value = value.to_string();
            v.enabled = true;
        }
        None => list.push(KeyValue::new(key, value)),
    }
}

// ---------------------------------------------------------------------------------------------
// Postman format
// ---------------------------------------------------------------------------------------------

fn str_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Postman descriptions are either a string or `{ "content": "..." }`.
fn description_of(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(o)) => o.get("content").map(str_of).unwrap_or_default(),
        _ => String::new(),
    }
}

fn url_to_string(url: &Value) -> String {
    match url {
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            if let Some(raw) = o.get("raw").and_then(Value::as_str) {
                return raw.to_string();
            }
            let join = |v: Option<&Value>, sep: &str| match v {
                Some(Value::Array(a)) => a.iter().map(str_of).collect::<Vec<_>>().join(sep),
                Some(v) => str_of(v),
                None => String::new(),
            };
            let mut s = String::new();
            if let Some(p) = o.get("protocol").and_then(Value::as_str) {
                s.push_str(p);
                s.push_str("://");
            }
            s.push_str(&join(o.get("host"), "."));
            if let Some(port) = o.get("port") {
                s.push(':');
                s.push_str(&str_of(port));
            }
            let path = join(o.get("path"), "/");
            if !path.is_empty() {
                s.push('/');
                s.push_str(&path);
            }
            let query: Vec<String> = o
                .get("query")
                .and_then(Value::as_array)
                .map(|q| {
                    q.iter()
                        .filter(|x| !x["disabled"].as_bool().unwrap_or(false))
                        .map(|x| format!("{}={}", str_of(&x["key"]), str_of(&x["value"])))
                        .collect()
                })
                .unwrap_or_default();
            if !query.is_empty() {
                s.push('?');
                s.push_str(&query.join("&"));
            }
            s
        }
        _ => String::new(),
    }
}

fn kv_list(v: Option<&Value>) -> Vec<KeyValue> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|x| {
                    let file = x["type"].as_str() == Some("file");
                    let value = if file {
                        match &x["src"] {
                            Value::Array(a) => a.first().map(str_of).unwrap_or_default(),
                            v => str_of(v),
                        }
                    } else {
                        str_of(&x["value"])
                    };
                    KeyValue {
                        key: str_of(&x["key"]),
                        value,
                        enabled: !x["disabled"].as_bool().unwrap_or(false),
                        file,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_auth(a: Option<&Value>) -> Auth {
    let Some(a) = a else { return Auth::Inherit };
    let kind = a["type"].as_str().unwrap_or("");
    // Values are `[{key, value}]` in v2.1 and `{key: value}` in v2.0.
    let get = |k: &str| -> String {
        match &a[kind] {
            Value::Array(list) => list
                .iter()
                .find(|x| x["key"].as_str() == Some(k))
                .map(|x| str_of(&x["value"]))
                .unwrap_or_default(),
            Value::Object(o) => o.get(k).map(str_of).unwrap_or_default(),
            _ => String::new(),
        }
    };
    match kind {
        "noauth" => Auth::None,
        "bearer" => Auth::Bearer {
            token: get("token"),
        },
        "basic" => Auth::Basic {
            username: get("username"),
            password: get("password"),
        },
        "apikey" => Auth::ApiKey {
            key: get("key"),
            value: get("value"),
            in_query: get("in") == "query",
        },
        "oauth2" => {
            let prefix = get("headerPrefix");
            Auth::OAuth2 {
                access_token: get("accessToken"),
                header_prefix: if prefix.is_empty() {
                    "Bearer".into()
                } else {
                    prefix
                },
                token_url: get("accessTokenUrl"),
                client_id: get("clientId"),
                client_secret: get("clientSecret"),
                scope: get("scope"),
                password_grant: get("grant_type") == "password_credentials",
                username: get("username"),
                password: get("password"),
            }
        }
        "digest" => Auth::Digest {
            username: get("username"),
            password: get("password"),
        },
        "awsv4" => Auth::Aws {
            access_key: get("accessKey"),
            secret_key: get("secretKey"),
            region: get("region"),
            service: get("service"),
            session_token: get("sessionToken"),
        },
        "ntlm" => Auth::Ntlm {
            username: get("username"),
            password: get("password"),
            domain: get("domain"),
            workstation: get("workstation"),
        },
        _ => Auth::Inherit,
    }
}

const GENERATED_START: &str = "// --- Gaza Man tests (generated) ---";
const GENERATED_END: &str = "// --- end Gaza Man tests ---";

/// Removes the test lines we generated on export, so re-importing doesn't double them.
fn strip_generated(events: Vec<Value>) -> Vec<Value> {
    events
        .into_iter()
        .filter_map(|mut e| {
            let Some(exec) = e["script"]["exec"].as_array() else {
                return Some(e);
            };
            let mut keep = Vec::new();
            let mut inside = false;
            for line in exec {
                let s = line.as_str().unwrap_or("");
                if s == GENERATED_START {
                    inside = true;
                } else if s == GENERATED_END {
                    inside = false;
                } else if !inside {
                    keep.push(line.clone());
                }
            }
            if keep
                .iter()
                .all(|l| l.as_str().is_some_and(|s| s.trim().is_empty()))
            {
                return None;
            }
            e["script"]["exec"] = Value::Array(keep);
            Some(e)
        })
        .collect()
}

fn events_of(v: Option<&Value>) -> Vec<Value> {
    strip_generated(v.and_then(Value::as_array).cloned().unwrap_or_default())
}

fn parse_request(it: &Value, name: String) -> Request {
    let req = &it["request"];
    // A request can also be just a URL string.
    let (method, url_value) = match req {
        Value::String(s) => ("GET".to_string(), Value::String(s.clone())),
        _ => (
            req["method"].as_str().unwrap_or("GET").to_uppercase(),
            req["url"].clone(),
        ),
    };
    let url = url_to_string(&url_value);

    let mut r = Request {
        name,
        method,
        url,
        ..Default::default()
    };
    // Prefer the query list (it keeps disabled params); fall back to the raw URL.
    match url_value.get("query").and_then(Value::as_array) {
        Some(_) => r.params = kv_list(url_value.get("query")),
        None => r.sync_from_url(),
    }
    let saved_vars = kv_list(url_value.get("variable"));
    r.sync_path_vars();
    for v in &mut r.path_vars {
        if let Some(saved) = saved_vars.iter().find(|s| s.key == v.key) {
            v.value = saved.value.clone();
        }
    }

    r.headers = kv_list(req.get("header"));
    let body = &req["body"];
    match body["mode"].as_str() {
        Some("raw") => {
            r.body_type = BodyType::Raw;
            r.raw = str_of(&body["raw"]);
            r.raw_lang = match body["options"]["raw"]["language"].as_str() {
                Some(l) => RawLang::from_postman(l),
                None if serde_json::from_str::<Value>(&r.raw).is_ok() => RawLang::Json,
                None => RawLang::Text,
            };
        }
        Some("urlencoded") => {
            r.body_type = BodyType::UrlEncoded;
            r.urlencoded = kv_list(body.get("urlencoded"));
        }
        Some("formdata") => {
            r.body_type = BodyType::FormData;
            r.form = kv_list(body.get("formdata"));
        }
        Some("file") => {
            r.body_type = BodyType::Binary;
            r.binary_path = str_of(&body["file"]["src"]);
        }
        Some("graphql") => {
            r.body_type = BodyType::GraphQl;
            let g = &body["graphql"];
            r.gql_query = str_of(&g["query"]);
            r.gql_variables = match &g["variables"] {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                v => serde_json::to_string_pretty(v).unwrap_or_default(),
            };
        }
        _ => {}
    }
    r.auth = parse_auth(req.get("auth"));
    r.description = description_of(req.get("description"));
    r.events = events_of(it.get("event"));
    r.examples = it
        .get("response")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|e| Example {
                    id: new_id(),
                    name: e["name"].as_str().unwrap_or("Example").to_string(),
                    status: e["code"].as_u64().unwrap_or(200) as u16,
                    headers: kv_list(e.get("header")),
                    body: str_of(&e["body"]),
                })
                .collect()
        })
        .unwrap_or_default();
    if let Some(ext) = it.get("_gazaman") {
        r.tests = serde_json::from_value(ext["tests"].clone()).unwrap_or_default();
        r.captures = serde_json::from_value(ext["captures"].clone()).unwrap_or_default();
        r.kind = serde_json::from_value(ext["kind"].clone()).unwrap_or_default();
        r.ws_message = ext["ws_message"].as_str().unwrap_or_default().to_string();
        if let Some(e) = ext["sio_event"].as_str() {
            r.sio_event = e.to_string();
        }
    }
    r
}

fn parse_items(items: &[Value]) -> Vec<Node> {
    items
        .iter()
        .map(|it| {
            let name = it["name"].as_str().unwrap_or("Untitled").to_string();
            match it.get("item").and_then(Value::as_array) {
                Some(children) => Node::Folder(Folder {
                    id: new_id(),
                    name,
                    description: description_of(it.get("description")),
                    auth: parse_auth(it.get("auth")),
                    items: parse_items(children),
                    events: events_of(it.get("event")),
                }),
                None => Node::Request(parse_request(it, name)),
            }
        })
        .collect()
}

pub fn is_postman_collection(json: &Value) -> bool {
    json.get("item").is_some_and(Value::is_array)
}

pub fn parse(json: &Value) -> Result<Collection, String> {
    let items = json
        .get("item")
        .and_then(Value::as_array)
        .ok_or("This file is not a Postman collection (no \"item\" list)")?;
    let info = &json["info"];
    let auth = match parse_auth(json.get("auth")) {
        Auth::Inherit => Auth::None,
        a => a,
    };
    Ok(Collection {
        id: new_id(),
        name: info["name"].as_str().unwrap_or("Collection").to_string(),
        description: description_of(info.get("description")),
        auth,
        variables: kv_list(json.get("variable")),
        items: parse_items(items),
        events: events_of(json.get("event")),
    })
}

fn auth_to_json(a: &Auth) -> Option<Value> {
    let kv = |pairs: &[(&str, &str)]| -> Value {
        pairs
            .iter()
            .map(|(k, v)| json!({ "key": k, "value": v, "type": "string" }))
            .collect()
    };
    Some(match a {
        Auth::Inherit => return None,
        Auth::None => json!({ "type": "noauth" }),
        Auth::Bearer { token } => json!({ "type": "bearer", "bearer": kv(&[("token", token)]) }),
        Auth::Basic { username, password } => {
            json!({ "type": "basic", "basic": kv(&[("username", username), ("password", password)]) })
        }
        Auth::ApiKey {
            key,
            value,
            in_query,
        } => json!({
            "type": "apikey",
            "apikey": kv(&[("key", key), ("value", value), ("in", if *in_query { "query" } else { "header" })])
        }),
        Auth::OAuth2 {
            access_token,
            header_prefix,
            token_url,
            client_id,
            client_secret,
            scope,
            password_grant,
            username,
            password,
        } => {
            json!({
                "type": "oauth2",
                "oauth2": kv(&[
                    ("accessToken", access_token),
                    ("headerPrefix", header_prefix),
                    ("accessTokenUrl", token_url),
                    ("clientId", client_id),
                    ("clientSecret", client_secret),
                    ("scope", scope),
                    ("grant_type", if *password_grant { "password_credentials" } else { "client_credentials" }),
                    ("username", username),
                    ("password", password),
                    ("addTokenTo", "header"),
                ])
            })
        }
        Auth::Digest { username, password } => json!({
            "type": "digest",
            "digest": kv(&[("username", username), ("password", password), ("algorithm", "MD5")])
        }),
        Auth::Aws {
            access_key,
            secret_key,
            region,
            service,
            session_token,
        } => json!({
            "type": "awsv4",
            "awsv4": kv(&[
                ("accessKey", access_key),
                ("secretKey", secret_key),
                ("region", region),
                ("service", service),
                ("sessionToken", session_token),
            ])
        }),
        Auth::Ntlm {
            username,
            password,
            domain,
            workstation,
        } => json!({
            "type": "ntlm",
            "ntlm": kv(&[
                ("username", username),
                ("password", password),
                ("domain", domain),
                ("workstation", workstation),
            ])
        }),
    })
}

fn kv_to_json(list: &[KeyValue], with_type: bool) -> Value {
    list.iter()
        .map(|kv| {
            let mut o = Map::new();
            o.insert("key".into(), kv.key.clone().into());
            if kv.file {
                o.insert("type".into(), "file".into());
                o.insert("src".into(), kv.value.clone().into());
            } else {
                o.insert("value".into(), kv.value.clone().into());
                if with_type {
                    o.insert("type".into(), "text".into());
                }
            }
            if !kv.enabled {
                o.insert("disabled".into(), true.into());
            }
            Value::Object(o)
        })
        .collect()
}

/// The Postman url object: raw plus protocol / host / path / query, which Postman expects.
fn url_to_json(r: &Request) -> Value {
    let base = url_base(&r.url);
    let (protocol, rest) = match base.split_once("://") {
        Some((p, rest)) => (Some(p), rest),
        None => (None, base),
    };
    let (host_port, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = match host_port.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => (h, Some(p)),
        _ => (host_port, None),
    };
    let mut o = Map::new();
    o.insert("raw".into(), r.url.clone().into());
    if let Some(p) = protocol {
        o.insert("protocol".into(), p.into());
    }
    let host_parts: Vec<Value> = if host.starts_with("{{") {
        vec![host.into()]
    } else {
        host.split('.').map(Value::from).collect()
    };
    o.insert("host".into(), host_parts.into());
    if let Some(p) = port {
        o.insert("port".into(), p.into());
    }
    if !path.is_empty() {
        o.insert("path".into(), path.split('/').map(Value::from).collect());
    }
    if !r.params.is_empty() {
        o.insert("query".into(), kv_to_json(&r.params, false));
    }
    if !r.path_vars.is_empty() {
        o.insert("variable".into(), kv_to_json(&r.path_vars, false));
    }
    Value::Object(o)
}

fn request_to_json(r: &Request) -> Value {
    let mut request = json!({
        "method": r.method,
        "header": kv_to_json(&r.headers, false),
        "url": url_to_json(r),
    });
    if let Some(a) = auth_to_json(&r.auth) {
        request["auth"] = a;
    }
    match r.body_type {
        BodyType::Raw => {
            request["body"] = json!({ "mode": "raw", "raw": r.raw, "options": { "raw": { "language": r.raw_lang.postman() } } });
        }
        BodyType::UrlEncoded => {
            request["body"] =
                json!({ "mode": "urlencoded", "urlencoded": kv_to_json(&r.urlencoded, true) })
        }
        BodyType::FormData => {
            request["body"] = json!({ "mode": "formdata", "formdata": kv_to_json(&r.form, true) })
        }
        BodyType::Binary => {
            request["body"] = json!({ "mode": "file", "file": { "src": r.binary_path } })
        }
        BodyType::GraphQl => {
            request["body"] = json!({ "mode": "graphql", "graphql": { "query": r.gql_query, "variables": r.gql_variables } })
        }
        BodyType::None => {}
    }
    if !r.description.is_empty() {
        request["description"] = r.description.clone().into();
    }

    let mut item = json!({ "name": r.name, "request": request });
    let generated = crate::tests::postman_script(&r.tests, &r.captures);
    let mut events = r.events.clone();
    if !generated.is_empty() {
        let mut lines: Vec<Value> = vec![GENERATED_START.into()];
        lines.extend(generated.lines().map(Value::from));
        lines.push(GENERATED_END.into());
        match events.iter_mut().find(|e| e["listen"] == "test") {
            Some(e) => {
                if let Some(exec) = e["script"]["exec"].as_array_mut() {
                    exec.extend(lines);
                }
            }
            None => events.push(
                json!({ "listen": "test", "script": { "type": "text/javascript", "exec": lines } }),
            ),
        }
        item["_gazaman"] = json!({ "tests": r.tests, "captures": r.captures });
    }
    if r.kind != RequestKind::Http {
        if !item["_gazaman"].is_object() {
            item["_gazaman"] = json!({});
        }
        item["_gazaman"]["kind"] = json!(r.kind);
        item["_gazaman"]["ws_message"] = json!(r.ws_message);
        item["_gazaman"]["sio_event"] = json!(r.sio_event);
    }
    if !events.is_empty() {
        item["event"] = Value::Array(events);
    }
    if !r.examples.is_empty() {
        let original = item["request"].clone();
        item["response"] = r
            .examples
            .iter()
            .map(|e| {
                let lang = match e.content_type() {
                    ct if ct.contains("json") => "json",
                    ct if ct.contains("html") => "html",
                    ct if ct.contains("xml") => "xml",
                    _ => "text",
                };
                json!({
                    "name": e.name,
                    "originalRequest": original,
                    "status": crate::http::reason(e.status),
                    "code": e.status,
                    "_postman_previewlanguage": lang,
                    "header": kv_to_json(&e.headers, false),
                    "cookie": [],
                    "body": e.body,
                })
            })
            .collect();
    }
    item
}

fn items_to_json(items: &[Node]) -> Value {
    items
        .iter()
        .map(|n| match n {
            Node::Request(r) => request_to_json(r),
            Node::Folder(f) => {
                let mut o = json!({ "name": f.name, "item": items_to_json(&f.items) });
                if !f.description.is_empty() {
                    o["description"] = f.description.clone().into();
                }
                if let Some(a) = auth_to_json(&f.auth) {
                    o["auth"] = a;
                }
                if !f.events.is_empty() {
                    o["event"] = Value::Array(f.events.clone());
                }
                o
            }
        })
        .collect()
}

/// Writes the collection as Postman v2.1 JSON.
pub fn to_json(c: &Collection) -> Value {
    let mut out = json!({
        "info": {
            "_postman_id": uuid_from(c.id),
            "name": c.name,
            "description": c.description,
            "schema": "https://schema.getpostman.com/json/collection/v2.1.0/collection.json",
        },
        "item": items_to_json(&c.items),
        "variable": kv_to_json(&c.variables, false),
    });
    if let Some(a) = auth_to_json(&c.auth).filter(|_| c.auth != Auth::None) {
        out["auth"] = a;
    }
    if !c.events.is_empty() {
        out["event"] = Value::Array(c.events.clone());
    }
    out
}

fn uuid_from(id: Id) -> String {
    let hex = format!("{:032x}", (id as u128) << 64 | (id.rotate_left(17) as u128));
    format!(
        "{}-{}-4{}-a{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..]
    )
}

pub fn is_postman_environment(json: &Value) -> bool {
    json.get("values").is_some_and(Value::is_array) && json.get("item").is_none()
}

pub fn parse_environment(json: &Value) -> Result<Environment, String> {
    if !is_postman_environment(json) {
        return Err("This file is not a Postman environment (no \"values\" list)".into());
    }
    Ok(Environment {
        id: new_id(),
        name: json["name"].as_str().unwrap_or("Environment").to_string(),
        values: kv_list(json.get("values")),
    })
}

pub fn environment_to_json(e: &Environment) -> Value {
    json!({
        "id": uuid_from(e.id),
        "name": e.name,
        "values": e.values.iter().map(|v| json!({ "key": v.key, "value": v.value, "type": "default", "enabled": v.enabled })).collect::<Vec<_>>(),
        "_postman_variable_scope": "environment",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_round_trip() {
        let mut r = Request {
            url: "{{base}}/users/:id?page=2&q=a#top".into(),
            ..Default::default()
        };
        r.sync_from_url();
        assert_eq!(r.params.len(), 2);
        assert_eq!(r.path_vars.len(), 1);
        assert_eq!(r.path_vars[0].key, "id");
        r.params[0].enabled = false;
        r.sync_to_url();
        assert_eq!(r.url, "{{base}}/users/:id?q=a#top");
        r.sync_from_url();
        assert_eq!(r.params.len(), 2, "disabled params are kept");
    }

    #[test]
    fn postman_round_trip() {
        let mut c = Collection::new("Demo");
        let mut r = Request::new("Login");
        r.method = "POST".into();
        r.url = "https://api.example.com:8080/v1/login?x=1".into();
        r.sync_from_url();
        r.body_type = BodyType::Raw;
        r.raw = "{\"a\":1}".into();
        r.tests.push(Assertion::default());
        r.examples.push(Example { name: "Ok".into(), status: 201, body: "{\"token\":\"x\"}".into(), ..Default::default() });
        r.captures.push(Capture {
            var: "token".into(),
            path: "token".into(),
            ..Default::default()
        });
        let mut f = Folder {
            name: "Auth".into(),
            ..Default::default()
        };
        f.items.push(Node::Request(r));
        c.items.push(Node::Folder(f));

        let json = to_json(&c);
        assert_eq!(json["item"][0]["item"][0]["request"]["url"]["port"], "8080");
        let back = parse(&json).unwrap();
        let req = back.requests()[0];
        assert_eq!(req.name, "Login");
        assert_eq!(req.url, "https://api.example.com:8080/v1/login?x=1");
        assert_eq!(req.tests.len(), 1);
        assert_eq!(req.captures.len(), 1);
        assert_eq!(req.examples.len(), 1);
        assert_eq!(req.examples[0].status, 201);
        assert_eq!(req.examples[0].content_type(), "application/json");
        assert!(
            req.events.is_empty(),
            "generated script is not kept as a foreign script"
        );
        // Exporting again doesn't duplicate the generated script.
        let again = to_json(&back);
        let exec = again["item"][0]["item"][0]["event"][0]["script"]["exec"]
            .as_array()
            .unwrap();
        assert_eq!(
            exec.iter()
                .filter(|l| l.as_str() == Some(GENERATED_START))
                .count(),
            1
        );
    }

    #[test]
    fn tree_operations() {
        let mut c = Collection::new("C");
        let mut f = Folder::default();
        let r = Request::new("A");
        let rid = r.id;
        f.items.push(Node::Request(r));
        let fid = f.id;
        c.items.push(Node::Folder(f));
        assert_eq!(c.path_to(rid).len(), 1);
        let copy = c.duplicate(rid).unwrap();
        assert_eq!(c.count_requests(), 2);
        assert_eq!(c.request(copy).unwrap().name, "A Copy");
        c.remove(rid);
        assert_eq!(c.requests_in(Some(fid)).len(), 1);
    }
}
