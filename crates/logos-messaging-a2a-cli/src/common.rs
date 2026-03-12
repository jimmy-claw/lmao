pub fn parse_capabilities(capabilities: &str) -> Vec<String> {
    capabilities
        .split(',')
        .map(|s| s.trim().to_string())
        .collect()
}
