//! Service inference tests.

use super::listeners::*;
    use super::*;

    fn spec(program: &str, args: &[&str], cwd: &Path) -> TaskSpec {
        TaskSpec {
            label: "service".to_string(),
            program: program.to_string(),
            args: args.iter().map(|value| value.to_string()).collect(),
            cwd: cwd.to_path_buf(),
            env: BTreeMap::new(),
            shell: false,
            auto_start: false,
            stop_timeout_ms: 3_000,
            clear_logs_on_restart: false,

            schedule: None,
        }
    }

    #[test]
    fn identifies_vite_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vite":"latest"},"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let technology = infer_service(&spec("npm", &["run", "dev"], dir.path())).technology;
        assert_eq!(technology.runtime.as_deref(), Some("node"));
        assert_eq!(technology.framework.as_deref(), Some("vite"));
        assert_eq!(technology.confidence, ServiceConfidence::High);
    }

    #[test]
    fn parses_lsof_ipv4_wildcard_and_ipv6_listeners() {
        let endpoints = parse_lsof_fields("p42\nn*:5173\np43\nn[::1]:8000\n");
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].bind_host, "0.0.0.0");
        assert_eq!(endpoints[0].port, 5173);
        assert_eq!(endpoints[0].pid, Some(42));
        assert_eq!(endpoints[1].bind_host, "::1");
        assert_eq!(endpoints[1].port, 8000);
    }

    #[test]
    fn parses_powershell_tcp_listener_rows() {
        let endpoints = parse_powershell_tcp_listeners("42\t0.0.0.0\t5173\r\n43\t::1\t8000\r\n");

        assert_eq!(
            endpoints
                .iter()
                .map(|endpoint| (endpoint.pid, endpoint.bind_host.as_str(), endpoint.port))
                .collect::<Vec<_>>(),
            [(Some(42), "0.0.0.0", 5173), (Some(43), "::1", 8000)]
        );
    }

    #[test]
    fn parses_reported_http_url_without_marking_it_listening() {
        let endpoints =
            endpoints_from_logs(["ready at http://127.0.0.1:3000/", "other output"].into_iter());
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].protocol, "http");
        assert_eq!(endpoints[0].state, "reported");
    }
