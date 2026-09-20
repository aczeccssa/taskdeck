//! Atomic config write with rollback guard.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

use crate::protocol::{EditableTask, EditableTaskInput, EditableTaskOrigin, SessionConfigSnapshot};
use crate::state::validate_cron_expression;

use super::merge::*;
use super::vscode::*;
use super::yaml::*;
use super::*;
#[derive(Debug)]
pub enum WriteConfigError {
    StaleRevision { current_revision: String },

    Validation { message: String },

    Other(anyhow::Error),
}

pub struct PreparedSessionConfigWrite {
    pub(crate) project: PathBuf,

    pub(crate) source: String,

    pub(crate) expected_revision: String,

    pub(crate) vscode_tasks: BTreeMap<String, EditableTaskInput>,

    pub(crate) merged_tasks: BTreeMap<String, EditableTaskInput>,

    pub(crate) workspace_env: BTreeMap<String, String>,

    pub(crate) task_order: Vec<String>,

    pub(crate) yaml_task_labels: HashSet<String>,

    pub(crate) temp_path: PathBuf,

    pub(crate) finalized: bool,
}

impl WriteConfigError {
    pub(crate) fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }
}

impl PreparedSessionConfigWrite {
    pub fn project_definition(&self, session: &str) -> Result<ProjectDefinition> {
        validate_session_name(session)?;

        let tasks = self
            .merged_tasks
            .values()
            .map(|task| {
                Ok((
                    task.label.clone(),
                    compile_task(&self.project, task, &self.workspace_env).map_err(|error| {
                        anyhow::anyhow!("invalid task '{}': {error}", task.label)
                    })?,
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;

        Ok(ProjectDefinition {
            session: session.to_string(),

            project: self.project.clone(),

            source: self.source.clone(),

            tasks,

            task_order: self.task_order.clone(),
        })
    }

    pub fn session_snapshot(&self, session: &str) -> Result<SessionConfigSnapshot> {
        validate_session_name(session)?;

        let tasks = self
            .task_order
            .iter()
            .filter_map(|label| self.merged_tasks.get(label))
            .map(|task| EditableTask {
                label: task.label.clone(),

                command: task.command.clone(),

                args: task.args.clone(),

                cwd: task.cwd.clone(),

                env: task.env.clone(),

                shell: task.shell,

                auto_start: task.auto_start,

                stop_timeout_ms: task.stop_timeout_ms,

                clear_logs_on_restart: task.clear_logs_on_restart,

                schedule: task.schedule.clone(),

                origin: EditableTaskOrigin {
                    imported: self.vscode_tasks.contains_key(&task.label),

                    has_yaml_override: self.yaml_task_labels.contains(&task.label)
                        && self.vscode_tasks.contains_key(&task.label),
                },
            })
            .collect();

        Ok(SessionConfigSnapshot {
            session: session.to_string(),

            project: self.project.clone(),

            source: self.source.clone(),

            revision: revision_for_project(&self.project)?,

            workspace_env: self.workspace_env.clone(),

            tasks,
        })
    }

    pub fn finalize(&mut self) -> std::result::Result<(), WriteConfigError> {
        let current_revision =
            revision_for_project(&self.project).map_err(WriteConfigError::Other)?;

        if current_revision != self.expected_revision {
            return Err(WriteConfigError::StaleRevision { current_revision });
        }

        rename_temp_file(&self.temp_path, &self.project.join(PROJECT_CONFIG))
            .map_err(WriteConfigError::Other)?;

        sync_parent_directory(&self.project.join(PROJECT_CONFIG))
            .map_err(WriteConfigError::Other)?;

        self.finalized = true;

        Ok(())
    }
}

impl Drop for PreparedSessionConfigWrite {
    fn drop(&mut self) {
        if !self.finalized {
            let _ = fs::remove_file(&self.temp_path);
        }
    }
}

pub(crate) fn write_temp_config_file(path: &Path, content: &str) -> Result<PathBuf> {
    let parent = path
        .parent()
        .with_context(|| format!("{} has no parent directory", path.display()))?;

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let temp = parent.join(format!(".{}.{}.tmp", PROJECT_CONFIG, nonce));

    let result = (|| -> Result<PathBuf> {
        let mut options = OpenOptions::new();

        options.create_new(true).write(true).truncate(true);

        #[cfg(unix)]
        options.mode(0o600);

        let mut file = options
            .open(&temp)
            .with_context(|| format!("failed to create {}", temp.display()))?;

        file.write_all(content.as_bytes())
            .with_context(|| format!("failed to write {}", temp.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = file.metadata()?.permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(&temp, permissions)
                .with_context(|| format!("failed to restrict permissions on {}", temp.display()))?;
        }

        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp.display()))?;

        Ok(temp.clone())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }

    result
}

pub(crate) fn sync_parent_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .with_context(|| format!("{} has no parent directory", path.display()))?;

        File::open(parent)
            .with_context(|| format!("failed to open {}", parent.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync {}", parent.display()))?;
    }

    #[cfg(not(unix))]
    let _ = path;

    Ok(())
}

pub(crate) fn revision_for_project(project: &Path) -> Result<String> {
    load_project_state(project).map(|state| state.revision())
}

#[cfg(unix)]

pub(crate) fn rename_temp_file(temp: &Path, path: &Path) -> Result<()> {
    match fs::rename(temp, path) {
        Ok(()) => Ok(()),

        Err(error) => {
            let _ = fs::remove_file(temp);

            Err(error).with_context(|| {
                format!(
                    "failed to replace {} with {}",
                    path.display(),
                    temp.display()
                )
            })
        }
    }
}

#[cfg(windows)]

pub(crate) fn rename_temp_file(temp: &Path, path: &Path) -> Result<()> {
    let temp_wide = temp
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();

    let replaced = unsafe {
        MoveFileExW(
            temp_wide.as_ptr(),
            path_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };

    if replaced != 0 {
        return Ok(());
    }

    let error = std::io::Error::last_os_error();

    let _ = fs::remove_file(temp);

    Err(error).with_context(|| {
        format!(
            "failed to replace {} with {}",
            path.display(),
            temp.display()
        )
    })
}
