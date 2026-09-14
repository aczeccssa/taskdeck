//! Service backend tests.

use super::linux_backend::render_systemd_unit;
use super::macos_backend::render_macos_plist;
use super::windows_backend::render_windows_command;
use super::*;

fn spec() -> ServiceSpec {
    ServiceSpec::new("/usr/local/bin/taskdeck", "/tmp/taskdeck home")
}

#[test]
fn renders_macos_plist_with_foreground_daemon_and_home() {
    let plist = render_macos_plist(LABEL, &spec());
    assert!(plist.contains("<string>/usr/local/bin/taskdeck</string>"));
    assert!(plist.contains("<string>daemon</string>"));
    assert!(plist.contains("TASKDECK_HOME"));
    assert!(plist.contains("<key>PATH</key><string>/usr/local/bin:/usr/bin:/bin</string>"));
    assert!(plist.contains("/tmp/taskdeck home"));
    assert!(!plist.contains("--background"));
}

#[test]
fn renders_systemd_user_and_system_units() {
    let user = render_systemd_unit(&spec(), ServiceScope::User);
    let system = render_systemd_unit(&spec(), ServiceScope::System);
    assert!(user.contains("WantedBy=default.target"));
    assert!(system.contains("WantedBy=multi-user.target"));
    assert!(user.contains("Environment=\"TASKDECK_HOME="));
    assert!(!user.contains("TASKDECK_HOME='"));
    assert!(user.contains("TASKDECK_HOME=/tmp/taskdeck home\""));
    assert!(!user.contains("--background"));
}

#[test]
fn extracts_home_from_all_service_file_formats() {
    let linux = "Environment='TASKDECK_HOME=/opt/taskdeck'\n";
    let macos = "<key>TASKDECK_HOME</key><string>/Users/dev/.taskdeck</string>\n";
    let windows = "set \"TASKDECK_HOME=C:\\Taskdeck\"\r\n";
    assert_eq!(
        extract_environment_home(linux).as_deref(),
        Some("/opt/taskdeck")
    );
    assert_eq!(
        extract_environment_home(macos).as_deref(),
        Some("/Users/dev/.taskdeck")
    );
    assert_eq!(
        extract_environment_home(windows).as_deref(),
        Some("C:\\Taskdeck")
    );
}

#[test]
fn renders_windows_login_command_with_home() {
    let command = render_windows_command(&spec());
    assert!(command.contains("set \"TASKDECK_HOME=/tmp/taskdeck home\""));
    assert!(command.contains("\"/usr/local/bin/taskdeck\" daemon"));
}
