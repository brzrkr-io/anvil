// Docker integration (roadmap I81). Read container state via the docker CLI;
// mutations are allow-listed. Logs/exec run in a terminal pane (frontend), so
// they aren't here.

fn docker(args: &[&str]) -> Result<String, String> {
    let mut cmd = crate::shared::command("docker");
    cmd.args(args);
    let out = crate::shared::exec_capture(cmd, 25).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "docker not found in PATH".to_string()
        } else {
            e.to_string()
        }
    })?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// List containers as newline-delimited JSON (`docker ps --format json`).
/// `all` includes stopped containers. The frontend parses + sorts.
#[tauri::command]
pub async fn docker_ps(all: bool) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut args = vec!["ps", "--no-trunc", "--format", "{{json .}}"];
        if all {
            args.push("-a");
        }
        docker(&args)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Allow-listed lifecycle action on a container: start | stop | restart | rm.
#[tauri::command]
pub async fn docker_action(id: String, action: String) -> Result<String, String> {
    let verb = match action.as_str() {
        "start" => "start",
        "stop" => "stop",
        "restart" => "restart",
        "rm" => "rm",
        _ => return Err(format!("invalid docker action: {action}")),
    };
    tauri::async_runtime::spawn_blocking(move || {
        // `rm -f` so a running container can be removed in one step.
        if verb == "rm" {
            docker(&["rm", "-f", &id])
        } else {
            docker(&[verb, &id])
        }
    })
    .await
    .map_err(|e| e.to_string())?
}
