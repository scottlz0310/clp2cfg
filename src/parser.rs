//! Loose JSON parser for MCP snippets.

use crate::model::McpServersMap;

/// Parse arbitrary MCP snippet strings into a normalized servers map.
pub fn parse_mcp_snippet(_input: &str) -> Result<McpServersMap, String> {
    // Skeleton implementation for initial project setup.
    // Full implementation will be added in Phase 1.
    Ok(McpServersMap::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skeleton_parser() {
        let res = parse_mcp_snippet("{}");
        assert!(res.is_ok());
    }
}
