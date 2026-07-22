use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

pub const PROJECT_CONFIG: &str = "taskdeck.yaml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSpec {
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
    pub shell: bool,
    pub auto_start: bool,
    pub stop_timeout_ms: u64,
}

impl TaskSpec {
    pub fn display_command(&self) -> String {
        std::iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone)]
pub struct ProjectDefinition {
    pub session: String,
    pub project: PathBuf,
    pub source: String,
    pub tasks: BTreeMap<String, TaskSpec>,
}

#[derive(Debug, Default, Deserialize)]
struct VscodeFile {
    #[serde(default)]
    tasks: Vec<VscodeTask>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VscodeTask {
    label: String,
    #[serde(rename = "type", default)]
    kind: String,
    command: String,
    #[serde(default)]
    args: Vec<JsonArg>,
    #[serde(default)]
    options: VscodeOptions,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum JsonArg {
    Text(String),
    Number(serde_json::Number),
    Bool(bool),
}

impl JsonArg {
    fn render(&self) -> String {
        match self {
            Self::Text(value) => value.clone(),
            Self::Number(value) => value.to_string(),
            Self::Bool(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct VscodeOptions {
    cwd: Option<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

#[derive(Debug, Default, Deserialize)]
struct YamlConfig {
    #[serde(default = "default_version")]
    version: u32,
    session: Option<String>,
    #[serde(default)]
    tasks: BTreeMap<String, TaskOverride>,
}

#[derive(Debug, Default, Deserialize)]
struct TaskOverride {
    enabled: Option<bool>,
    command: Option<String>,
    args: Option<Vec<String>>,
    cwd: Option<String>,
    env: Option<BTreeMap<String, String>>,
    shell: Option<bool>,
    auto_start: Option<bool>,
    stop_timeout_ms: Option<u64>,
}

fn default_version() -> u32 {
    1
}

pub fn discover(project: &Path, requested_session: Option<&str>) -> Result<ProjectDefinition> {
    let project = project
        .canonicalize()
        .with_context(|| format!("project directory does not exist: {}", project.display()))?;
    let mut tasks = load_vscode_tasks(&project)?;
    let yaml_path = project.join(PROJECT_CONFIG);
    let yaml = if yaml_path.exists() {
        let content = fs::read_to_string(&yaml_path)
            .with_context(|| format!("failed to read {}", yaml_path.display()))?;
        let config: YamlConfig = serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse {}", yaml_path.display()))?;
        if config.version != 1 {
            bail!(
                "unsupported taskdeck.yaml version {}; expected 1",
                config.version
            );
        }
        Some(config)
    } else {
        None
    };

    if let Some(config) = &yaml {
        apply_overrides(&project, &mut tasks, &config.tasks)?;
    }
    if tasks.is_empty() {
        bail!(
            "no tasks found; add .vscode/tasks.json or {}",
            yaml_path.display()
        );
    }

    let default_session = project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");
    let session = requested_session
        .map(str::to_owned)
        .or_else(|| yaml.as_ref().and_then(|config| config.session.clone()))
        .unwrap_or_else(|| default_session.to_owned());
    validate_session_name(&session)?;

    let source = match (project.join(".vscode/tasks.json").exists(), yaml.is_some()) {
        (true, true) => ".vscode/tasks.json + taskdeck.yaml",
        (true, false) => ".vscode/tasks.json",
        (false, true) => "taskdeck.yaml",
        (false, false) => unreachable!(),
    };

    Ok(ProjectDefinition {
        session,
        project,
        source: source.to_string(),
        tasks,
    })
}

fn load_vscode_tasks(project: &Path) -> Result<BTreeMap<String, TaskSpec>> {
    let path = project.join(".vscode/tasks.json");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let file: VscodeFile =
        json5::from_str(&content).with_context(|| format!("failed to parse {}", path.display()))?;
    let mut tasks = BTreeMap::new();
    for task in file.tasks {
        let shell = task.kind == "shell";
        let cwd = task
            .options
            .cwd
            .as_deref()
            .map(|cwd| expand(cwd, project))
            .map(PathBuf::from)
            .unwrap_or_else(|| project.to_path_buf());
        let args = task
            .args
            .iter()
            .map(JsonArg::render)
            .map(|arg| expand(&arg, project))
            .collect();
        let env = task
            .options
            .env
            .into_iter()
            .map(|(key, value)| (key, expand(&value, project)))
            .collect();
        let spec = TaskSpec {
            label: task.label.clone(),
            program: expand(&task.command, project),
            args,
            cwd,
            env,
            shell,
            auto_start: false,
            stop_timeout_ms: 3_000,
        };
        tasks.insert(task.label, spec);
    }
    Ok(tasks)
}

fn apply_overrides(
    project: &Path,
    tasks: &mut BTreeMap<String, TaskSpec>,
    overrides: &BTreeMap<String, TaskOverride>,
) -> Result<()> {
    for (label, patch) in overrides {
        if patch.enabled == Some(false) {
            tasks.remove(label);
            continue;
        }
        if !tasks.contains_key(label) && patch.command.is_none() {
            bail!("new YAML task '{label}' must define command");
        }
        let task = tasks.entry(label.clone()).or_insert_with(|| TaskSpec {
            label: label.clone(),
            program: String::new(),
            args: Vec::new(),
            cwd: project.to_path_buf(),
            env: BTreeMap::new(),
            shell: true,
            auto_start: false,
            stop_timeout_ms: 3_000,
        });
        if let Some(command) = &patch.command {
            task.program = expand(command, project);
        }
        if let Some(args) = &patch.args {
            task.args = args.iter().map(|arg| expand(arg, project)).collect();
        }
        if let Some(cwd) = &patch.cwd {
            let cwd = PathBuf::from(expand(cwd, project));
            task.cwd = if cwd.is_absolute() {
                cwd
            } else {
                project.join(cwd)
            };
        }
        if let Some(env) = &patch.env {
            task.env.extend(
                env.iter()
                    .map(|(key, value)| (key.clone(), expand(value, project))),
            );
        }
        if let Some(shell) = patch.shell {
            task.shell = shell;
        }
        if let Some(auto_start) = patch.auto_start {
            task.auto_start = auto_start;
        }
        if let Some(timeout) = patch.stop_timeout_ms {
            if timeout == 0 {
                bail!("task '{label}' stop_timeout_ms must be positive");
            }
            task.stop_timeout_ms = timeout;
        }
    }
    Ok(())
}

fn expand(input: &str, project: &Path) -> String {
    let basename = project
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let mut output = input
        .replace("${workspaceFolderBasename}", basename)
        .replace("${workspaceFolder}", &project.to_string_lossy());
    let environment: HashMap<String, String> = env::vars().collect();
    for (key, value) in environment {
        output = output.replace(&format!("${{env:{key}}}"), &value);
    }
    output
}

pub fn validate_session_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 64
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        bail!("session name must be 1-64 ASCII letters, digits, '.', '-' or '_'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_vscode_tasks_and_yaml_overrides() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".vscode")).unwrap();
        fs::write(
            dir.path().join(".vscode/tasks.json"),
            r#"{
                // JSON with comments is valid in VS Code.
                "tasks": [{
                    "label": "api", "type": "process", "command": "dotnet",
                    "args": ["run", "--project", "${workspaceFolder}/api.csproj"],
                    "options": { "cwd": "${workspaceFolder}" }
                }],
            }"#,
        )
        .unwrap();
        fs::write(
            dir.path().join(PROJECT_CONFIG),
            "version: 1\nsession: demo\ntasks:\n  api:\n    auto_start: true\n  web:\n    command: npm\n    args: [run, dev]\n",
        )
        .unwrap();

        let definition = discover(dir.path(), None).unwrap();
        assert_eq!(definition.session, "demo");
        assert_eq!(definition.tasks.len(), 2);
        assert!(definition.tasks["api"].auto_start);
        assert!(definition.tasks["api"].args[2].ends_with("api.csproj"));
    }
}
