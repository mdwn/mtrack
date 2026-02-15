// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

/// Resolves the effective hostname for profile matching.
///
/// Priority:
/// 1. MTRACK_HOSTNAME environment variable (allows override for testing/deployment)
/// 2. System hostname via gethostname()
pub fn resolve_hostname() -> String {
    if let Ok(h) = std::env::var("MTRACK_HOSTNAME") {
        if !h.is_empty() {
            return h;
        }
    }

    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hostname_env_override() {
        // Save original value to restore later.
        let original = std::env::var("MTRACK_HOSTNAME").ok();

        std::env::set_var("MTRACK_HOSTNAME", "test-host");
        assert_eq!(resolve_hostname(), "test-host");

        // Restore original.
        match original {
            Some(val) => std::env::set_var("MTRACK_HOSTNAME", val),
            None => std::env::remove_var("MTRACK_HOSTNAME"),
        }
    }

    #[test]
    fn test_hostname_empty_env_falls_back() {
        let original = std::env::var("MTRACK_HOSTNAME").ok();

        std::env::set_var("MTRACK_HOSTNAME", "");
        let hostname = resolve_hostname();
        // Should fall back to system hostname, which should not be empty.
        assert!(!hostname.is_empty());

        match original {
            Some(val) => std::env::set_var("MTRACK_HOSTNAME", val),
            None => std::env::remove_var("MTRACK_HOSTNAME"),
        }
    }
}
