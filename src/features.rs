//! Which parts of Gaza Man are shown. "Simple" keeps only what every request needs; "Full"
//! shows everything; and each feature can be switched on or off on its own.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Features {
    /// No-code checks on the response.
    pub tests: bool,
    /// Saving parts of a response into variables, without code.
    pub captures: bool,
    /// Pre-request and post-response JavaScript.
    pub scripts: bool,
    pub examples: bool,
    pub docs: bool,
    /// The `</>` code snippet panel.
    pub code: bool,
    pub runner: bool,
    pub environments: bool,
    pub history: bool,
    pub console: bool,
    pub cookies: bool,
    pub mock: bool,
    pub graphql: bool,
    /// WebSocket and Socket.IO requests.
    pub sockets: bool,
    /// Digest, AWS Signature, NTLM and OAuth 2.0, besides Bearer / Basic / API Key.
    pub advanced_auth: bool,
    /// The magnifying lens and the zoom buttons.
    pub teaching: bool,
}

impl Default for Features {
    fn default() -> Self {
        Self::SIMPLE
    }
}

pub struct Item {
    pub group: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub get: fn(&mut Features) -> &mut bool,
}

impl Features {
    pub const SIMPLE: Features = Features {
        tests: false,
        captures: false,
        scripts: false,
        examples: false,
        docs: false,
        code: false,
        runner: false,
        // Variables like {{baseUrl}} and {{token}} are part of everyday use, so they stay.
        environments: true,
        history: true,
        console: false,
        cookies: false,
        mock: false,
        graphql: false,
        sockets: false,
        advanced_auth: false,
        teaching: true,
    };

    pub const FULL: Features = Features {
        tests: true,
        captures: true,
        scripts: true,
        examples: true,
        docs: true,
        code: true,
        runner: true,
        environments: true,
        history: true,
        console: true,
        cookies: true,
        mock: true,
        graphql: true,
        sockets: true,
        advanced_auth: true,
        teaching: true,
    };

    pub fn items() -> [Item; 16] {
        [
            Item { group: "Request", label: "Tests", help: "Check the response without code (status, time, JSON values)", get: |f| &mut f.tests },
            Item { group: "Request", label: "Variables from responses", help: "Save a value from the response (like a token) for the next requests", get: |f| &mut f.captures },
            Item { group: "Request", label: "Scripts", help: "Pre-request and post-response JavaScript (pm.*)", get: |f| &mut f.scripts },
            Item { group: "Request", label: "Examples", help: "Saved responses, shown in docs and used by the mock server", get: |f| &mut f.examples },
            Item { group: "Request", label: "Docs", help: "A description for each request", get: |f| &mut f.docs },
            Item { group: "Request", label: "More auth types", help: "Digest, AWS Signature, NTLM and OAuth 2.0", get: |f| &mut f.advanced_auth },
            Item { group: "Request types", label: "GraphQL", help: "GraphQL body with schema suggestions", get: |f| &mut f.graphql },
            Item { group: "Request types", label: "WebSocket & Socket.IO", help: "Live connections that stay open", get: |f| &mut f.sockets },
            Item { group: "Tools", label: "Environments", help: "Switch between sets of variables, like Local and Production", get: |f| &mut f.environments },
            Item { group: "Tools", label: "History", help: "Every request you sent", get: |f| &mut f.history },
            Item { group: "Tools", label: "Collection Runner", help: "Run a whole collection or folder in order", get: |f| &mut f.runner },
            Item { group: "Tools", label: "Code snippets", help: "The request as code in 17 languages", get: |f| &mut f.code },
            Item { group: "Tools", label: "Console", help: "Each request exactly as sent, and script logs", get: |f| &mut f.console },
            Item { group: "Tools", label: "Cookies", help: "The cookie jar and the Cookies tab of a response", get: |f| &mut f.cookies },
            Item { group: "Tools", label: "Mock server", help: "Answer on localhost with saved examples", get: |f| &mut f.mock },
            Item { group: "Tools", label: "Teaching tools", help: "The magnifying lens and the zoom buttons", get: |f| &mut f.teaching },
        ]
    }

    pub fn is_simple(&self) -> bool {
        *self == Self::SIMPLE
    }

    pub fn is_full(&self) -> bool {
        *self == Self::FULL
    }

    /// How many features are on, out of all of them.
    pub fn count(&self) -> (usize, usize) {
        let mut copy = *self;
        let items = Self::items();
        let on = items.iter().filter(|i| *(i.get)(&mut copy)).count();
        (on, items.len())
    }
}
