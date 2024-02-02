// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
#[cfg(test)]
pub mod test {
    use std::{
        thread,
        time::{Duration, SystemTime},
    };

    /// Wait for the given predicate to return true or fail.
    #[inline]
    pub fn eventually<F>(predicate: F, error_msg: &str)
    where
        F: Fn() -> bool,
    {
        let start = SystemTime::now();
        let tick = Duration::from_millis(10);
        let timeout = Duration::from_secs(3);

        loop {
            let elapsed = start.elapsed();
            if elapsed.is_err() {
                assert!(false, "System time error");
            }
            let elapsed = elapsed.unwrap();

            if elapsed > timeout {
                assert!(false, "{}", error_msg);
            }
            if predicate() {
                return;
            }
            thread::sleep(tick);
        }
    }
}
