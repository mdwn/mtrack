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

use std::time::Duration;

/// Outputs the given duration in a minutes:seconds format.
pub fn duration_minutes_seconds(duration: Duration) -> String {
    let minutes = duration.as_secs() / 60;
    let secs = duration.as_secs() - minutes * 60;
    format!("{}:{:02}", minutes, secs)
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use crate::util::duration_minutes_seconds;

    #[test]
    fn test_duration_minutes_strings() {
        assert_eq!("0:00", duration_minutes_seconds(Duration::new(0, 0)));
        assert_eq!("0:05", duration_minutes_seconds(Duration::new(5, 0)));
        assert_eq!("0:55", duration_minutes_seconds(Duration::new(55, 0)));
        assert_eq!("1:00", duration_minutes_seconds(Duration::new(60, 0)));
        assert_eq!("2:05", duration_minutes_seconds(Duration::new(125, 0)));
        assert_eq!("60:06", duration_minutes_seconds(Duration::new(3606, 0)));
    }
}
