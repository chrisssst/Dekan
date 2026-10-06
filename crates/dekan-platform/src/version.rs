#[must_use]
pub fn display_version() -> &'static str {
    const RAW: &str = env!("CARGO_PKG_VERSION");
    if let Some(stripped) = RAW.strip_suffix(".0") {
        stripped
    } else {
        RAW
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_version() {
        let v = display_version();
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }
}
