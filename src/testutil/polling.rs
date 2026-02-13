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
    let mut tick = Duration::from_millis(5); // Start with shorter interval
    let timeout = Duration::from_secs(10); // Increased timeout for complex operations
    let max_tick = Duration::from_millis(100); // Cap the polling interval

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate() {
            return;
        }

        // Exponential backoff to reduce CPU contention
        thread::sleep(tick);
        tick = std::cmp::min(tick * 2, max_tick);
    }
}

/// Wait for the given async predicate to return true or fail.
#[inline]
pub async fn eventually_async<F, Fut>(mut predicate: F, error_msg: &str)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = SystemTime::now();
    let tick = Duration::from_millis(10);
    let timeout = Duration::from_secs(3);

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate().await {
            return;
        }
        tokio::time::sleep(tick).await;
    }
}
