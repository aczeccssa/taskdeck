//! Human-readable and machine-readable CLI output.

use anyhow::{Result, bail};
use serde_json::{Map, Value};

use crate::platform_service::ServiceStatus;
use crate::protocol::Response;

pub(crate) fn print_response(response: Response, json: bool) -> Result<()> {
    if !response.ok {
        bail!(response.message);
    }

    if json {
        if let Some(data) = response.data {
            println!("{}", serde_json::to_string_pretty(&data)?);
        } else {
            println!("{}", response.message);
        }
        return Ok(());
    }

    println!("{}", response.message);
    if let Some(data) = response.data {
        print_value(&data, "DATA");
    }
    Ok(())
}

pub(crate) fn print_service(status: Result<ServiceStatus>, json: bool) -> Result<()> {
    let status = status.map_err(|error| anyhow::anyhow!("{error:#}"))?;
    if json {
        println!("{}", serde_json::to_string_pretty(&status)?);
    } else {
        print_value(&serde_json::to_value(status)?, "SERVICE");
    }
    Ok(())
}

pub(crate) fn print_value(value: &Value, title: &str) {
    match value {
        Value::Array(values) => print_array(values, title),
        Value::Object(object) => print_object(object, title),
        scalar => print_table(title, &["VALUE"], &[vec![scalar_text(scalar)]]),
    }
}

pub(crate) fn print_message(message: &str) {
    print_table("RESULT", &["MESSAGE"], &[vec![message.to_string()]]);
}

pub(crate) fn print_table(title: &str, headers: &[&str], rows: &[Vec<String>]) {
    println!("{title}:");
    if rows.is_empty() {
        println!("(none)");
        return;
    }

    let widths = headers
        .iter()
        .enumerate()
        .map(|(index, header)| {
            std::iter::once(header.len())
                .chain(rows.iter().map(|row| row.get(index).map_or(0, String::len)))
                .max()
                .unwrap_or(header.len())
        })
        .collect::<Vec<_>>();
    let separator = widths
        .iter()
        .map(|width| "-".repeat(*width + 2))
        .collect::<Vec<_>>()
        .join("+");

    println!(
        "{}",
        format_row(headers.iter().map(|value| (*value).to_string()), &widths)
    );
    println!("{separator}");
    for row in rows {
        println!("{}", format_row(row.iter().cloned(), &widths));
    }
}

fn print_array(values: &[Value], title: &str) {
    if values.iter().all(Value::is_object) {
        let objects = values
            .iter()
            .filter_map(Value::as_object)
            .collect::<Vec<_>>();
        let mut columns = Vec::new();
        for object in &objects {
            for key in object.keys() {
                if !columns.contains(key) {
                    columns.push(key.clone());
                }
            }
        }
        let headers = columns.iter().map(String::as_str).collect::<Vec<_>>();
        let rows = objects
            .iter()
            .map(|object| {
                columns
                    .iter()
                    .map(|key| object.get(key).map_or_else(String::new, scalar_text))
                    .collect()
            })
            .collect::<Vec<Vec<_>>>();
        print_table(title, &headers, &rows);
    } else {
        let rows = values
            .iter()
            .map(|value| vec![scalar_text(value)])
            .collect::<Vec<_>>();
        print_table(title, &["VALUE"], &rows);
    }
}

fn print_object(object: &Map<String, Value>, title: &str) {
    if let (Some(name), Some(project), Some(tasks)) = (
        object.get("name"),
        object.get("project"),
        object.get("tasks"),
    ) {
        if tasks.is_object() {
            let mut summary = vec![
                vec!["NAME".to_string(), scalar_text(name)],
                vec!["PROJECT".to_string(), scalar_text(project)],
            ];
            for key in ["source", "alias", "revision"] {
                if let Some(value) = object.get(key) {
                    summary.push(vec![key.to_uppercase(), scalar_text(value)]);
                }
            }
            print_table(title, &["FIELD", "VALUE"], &summary);
            if let Some(tasks) = tasks.as_object() {
                print_tasks(tasks);
                print_task_logs(tasks);
            }
            return;
        }
    }

    if let Some(lines) = object.get("lines").and_then(Value::as_array) {
        let metadata = object
            .iter()
            .filter(|(key, value)| {
                key.as_str() != "lines" && !value.is_array() && !value.is_object()
            })
            .map(|(key, value)| vec![key.to_uppercase(), scalar_text(value)])
            .collect::<Vec<_>>();
        print_table(title, &["FIELD", "VALUE"], &metadata);
        print_array(lines, "LOGS");
        return;
    }

    let scalar_rows = object
        .iter()
        .filter(|(_, value)| !value.is_array() && !value.is_object())
        .map(|(key, value)| vec![key.to_uppercase(), scalar_text(value)])
        .collect::<Vec<_>>();
    print_table(title, &["FIELD", "VALUE"], &scalar_rows);
    for (key, value) in object {
        if value.is_array() {
            print_array(
                value.as_array().map_or(&[], |values| values),
                &key.to_uppercase(),
            );
        } else if value.is_object() {
            print_value(value, &key.to_uppercase());
        }
    }
}

fn print_tasks(tasks: &Map<String, Value>) {
    let rows = tasks
        .iter()
        .map(|(label, value)| {
            let object = value.as_object();
            vec![
                label.clone(),
                object
                    .and_then(|v| v.get("status"))
                    .map_or_else(String::new, scalar_text),
                object
                    .and_then(|v| v.get("pid"))
                    .map_or_else(String::new, scalar_text),
                object
                    .and_then(|v| v.get("command"))
                    .map_or_else(String::new, scalar_text),
                object
                    .and_then(|v| v.get("cwd"))
                    .map_or_else(String::new, scalar_text),
                object
                    .and_then(|v| v.get("auto_start"))
                    .map_or_else(String::new, scalar_text),
                object
                    .and_then(|v| v.get("last_exit"))
                    .map_or_else(String::new, scalar_text),
            ]
        })
        .collect::<Vec<_>>();
    print_table(
        "TASKS",
        &[
            "TASK",
            "STATUS",
            "PID",
            "COMMAND",
            "CWD",
            "AUTO START",
            "LAST EXIT",
        ],
        &rows,
    );
}

fn print_task_logs(tasks: &Map<String, Value>) {
    let rows = tasks
        .iter()
        .flat_map(|(task, value)| {
            value
                .get("logs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|line| line.as_object())
                .map(|line| {
                    vec![
                        task.clone(),
                        line.get("seq").map_or_else(String::new, scalar_text),
                        line.get("stream").map_or_else(String::new, scalar_text),
                        line.get("text").map_or_else(String::new, scalar_text),
                    ]
                })
        })
        .collect::<Vec<_>>();
    if !rows.is_empty() {
        print_table("LOGS", &["TASK", "SEQ", "STREAM", "TEXT"], &rows);
    }
}

fn scalar_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).unwrap_or_default(),
    }
}

fn format_row<I>(cells: I, widths: &[usize]) -> String
where
    I: IntoIterator<Item = String>,
{
    cells
        .into_iter()
        .enumerate()
        .map(|(index, cell)| format!(" {:width$} ", cell, width = widths[index]))
        .collect::<Vec<_>>()
        .join("|")
}
