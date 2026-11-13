//! The Mneme desktop app: window, and later commands, tray and shortcut (PRD 06).

/// Builds and runs the Tauri app. Blocks until the app exits.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("failed to run Mneme");
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    fn config() -> Value {
        serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json is JSON")
    }

    #[test]
    fn identity_matches_the_prds() {
        let config = config();
        assert_eq!(config["productName"], "Mneme");
        assert_eq!(config["identifier"], "app.mneme.desktop");
        assert_eq!(config["bundle"]["macOS"]["minimumSystemVersion"], "14.4");
        assert_eq!(config["app"]["windows"][0]["title"], "Mneme");
    }

    /// Local-only rule (PRD 00): the webview may load and connect to nothing
    /// outside the app itself.
    #[test]
    fn csp_allows_no_remote_source() {
        let config = config();
        let csp = config["app"]["security"]["csp"]
            .as_object()
            .expect("csp is set, as a map of directives");
        assert_eq!(csp["default-src"], "'self'");
        assert_eq!(csp["script-src"], "'self'");

        // Everything a directive may name. `ipc:` and `http://ipc.localhost`
        // are how the webview reaches the Rust side of this app.
        let allowed = ["'self'", "'none'", "data:", "ipc:", "http://ipc.localhost"];
        for (directive, sources) in csp {
            let sources = sources.as_str().expect("directive value is a string");
            for source in sources.split_whitespace() {
                assert!(
                    allowed.contains(&source),
                    "{directive} allows {source}, which is not on the local-only list"
                );
            }
        }
        assert!(config["app"]["security"].get("devCsp").is_none());
    }

    #[test]
    fn no_updater_and_no_plugins() {
        let config = config();
        assert!(config.get("plugins").is_none());
        assert!(config["bundle"].get("createUpdaterArtifacts").is_none());
    }
}
