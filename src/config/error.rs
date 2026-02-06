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

/// Typed error for config load/parse failures so callers can distinguish
/// e.g. file-not-found from parse errors without string matching.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("Config load/parse error: {0}")]
    Load(#[from] config::ConfigError),
}
