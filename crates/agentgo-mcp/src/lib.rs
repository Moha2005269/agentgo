//! Thin MCP adapter mapping (Milestone 9 deliverable, contract fixed now).
//!
//! Rule from the inventory: every adapter maps to the **same underlying
//! AgentGo commands, observations, permissions, and results**. No adapter
//! may silently remove control capabilities or add privileged ones.
//!
//! This crate holds the protocol [`client`] used by `agentgo call` and the
//! stdio MCP [`server`] behind `agentgo mcp`, plus the generic
//! `agentgo_<command_with_underscores>` naming rule and its validator.

pub mod client;
pub mod server;

use agentgo_protocol::PROTOCOL_VERSION;
use agentgo_shell_bridge::ShellMethodRegistry;

/// Errors of the mapping layer.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MappingError {
    /// The MCP tool name does not follow the naming rule.
    #[error("tool name {0:?} violates the agentgo_<command> naming rule")]
    BadName(String),
    /// The mapped protocol command does not exist.
    #[error("tool {tool:?} maps to unknown command {command:?}")]
    UnknownCommand {
        /// Offending tool name.
        tool: String,
        /// Unknown protocol command.
        command: String,
    },
}

/// Converts a dotted protocol command into its MCP tool name.
pub fn tool_name_for_command(command: &str) -> String {
    format!("agentgo_{}", command.replace('.', "_"))
}

/// Converts an MCP tool name back into the protocol command, validating
/// the naming rule.
pub fn command_for_tool_name(tool: &str) -> Result<String, MappingError> {
    let Some(rest) = tool.strip_prefix("agentgo_") else {
        return Err(MappingError::BadName(tool.to_string()));
    };
    if rest.is_empty()
        || !rest
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(MappingError::BadName(tool.to_string()));
    }
    Ok(rest.replace('_', "."))
}

/// Validates that a tool mapping points at a command that exists in the
/// shell bridge registry (the Milestone 0 command surface snapshot).
pub fn validate_mapping(tool: &str) -> Result<String, MappingError> {
    let command = command_for_tool_name(tool)?;
    let registry = ShellMethodRegistry::new();
    if registry.lookup(&command).is_none() {
        return Err(MappingError::UnknownCommand {
            tool: tool.to_string(),
            command,
        });
    }
    Ok(command)
}

/// The MCP adapter version report (same protocol version, no privileges
/// added or removed).
pub fn adapter_version() -> String {
    format!("agentgo-mcp adapter over protocol {PROTOCOL_VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_rule_roundtrip() {
        let tool = tool_name_for_command("tabs.navigate");
        assert_eq!(tool, "agentgo_tabs_navigate");
        assert_eq!(command_for_tool_name(&tool).unwrap(), "tabs.navigate");
    }

    #[test]
    fn bad_names_rejected() {
        assert!(matches!(
            command_for_tool_name("tabs.navigate"),
            Err(MappingError::BadName(_))
        ));
        assert!(matches!(
            command_for_tool_name("agentgo_"),
            Err(MappingError::BadName(_))
        ));
        assert!(matches!(
            command_for_tool_name("agentgo_Bad"),
            Err(MappingError::BadName(_))
        ));
        assert!(matches!(
            command_for_tool_name("other_x"),
            Err(MappingError::BadName(_))
        ));
    }

    #[test]
    fn mappings_must_point_at_real_commands() {
        assert_eq!(
            validate_mapping("agentgo_tabs_navigate").unwrap(),
            "tabs.navigate"
        );
        assert_eq!(
            validate_mapping("agentgo_agent_stop").unwrap(),
            "agent.stop"
        );
        match validate_mapping("agentgo_not_a_command") {
            Err(MappingError::UnknownCommand { tool, command }) => {
                assert_eq!(tool, "agentgo_not_a_command");
                assert_eq!(command, "not.a.command");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn adapter_reports_shared_protocol_version() {
        assert!(adapter_version().contains(agentgo_protocol::PROTOCOL_VERSION));
    }
}
