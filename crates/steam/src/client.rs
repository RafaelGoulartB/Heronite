//! The same settings through the running client (`SteamClient.Apps`), which
//! applies them at once and saves them itself.

use anyhow::{anyhow, Result};
use heronite_shortcuts::steam_client::SteamClient;
use serde_json::Value;

use crate::CompatTool;

/// What the client has for one game.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientApp {
    pub launch_options: String,
    /// `""`: Steam's default.
    pub compat_tool: String,
    /// Every tool the client offers for the game, Valve's first.
    pub tools: Vec<CompatTool>,
}

fn app_id_number(app_id: &str) -> Result<u32> {
    app_id.parse().map_err(|_| anyhow!("invalid Steam app id {app_id:?}"))
}

pub fn read(client: &SteamClient, app_id: &str) -> Result<ClientApp> {
    let id = app_id_number(app_id)?;
    // The details come through a callback; give up after a few seconds
    // (a game the account does not own never answers).
    let value = client.eval(&format!(
        r#"(async () => {{
      const details = await new Promise((resolve) => {{
        let done = false
        let reg = null
        const finish = (d) => {{ if (!done) {{ done = true; reg?.unregister(); resolve(d) }} }}
        reg = SteamClient.Apps.RegisterForAppDetails({id}, finish)
        if (done) reg.unregister()
        setTimeout(() => finish(null), 5000)
      }})
      if (!details) throw new Error('Steam has no details for app {id}')
      const tools = await SteamClient.Apps.GetAvailableCompatTools({id})
      return {{
        launchOptions: details.strLaunchOptions ?? '',
        compatTool: details.strCompatToolName ?? '',
        tools: (tools ?? []).map((t) => ({{ name: t.strToolName, display: t.strDisplayName }})),
      }}
    }})()"#
    ))?;
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let tools = value
        .get("tools")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter().map(|t| CompatTool { name: text(t, "name"), display: text(t, "display") }).filter(|t| !t.name.is_empty()).collect()
        })
        .unwrap_or_default();
    Ok(ClientApp { launch_options: text(&value, "launchOptions"), compat_tool: text(&value, "compatTool"), tools })
}

pub fn set_launch_options(client: &SteamClient, app_id: &str, line: &str) -> Result<()> {
    let id = app_id_number(app_id)?;
    let line = Value::String(line.to_string());
    client.eval(&format!("(async () => {{ SteamClient.Apps.SetAppLaunchOptions({id}, {line}); return true }})()"))?;
    Ok(())
}

/// `""` goes back to Steam's default.
pub fn set_compat_tool(client: &SteamClient, app_id: &str, tool: &str) -> Result<()> {
    let id = app_id_number(app_id)?;
    let tool = Value::String(tool.to_string());
    client.eval(&format!("(async () => {{ SteamClient.Apps.SpecifyCompatTool({id}, {tool}); return true }})()"))?;
    Ok(())
}
