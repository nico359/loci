/* location.rs
 *
 * Copyright 2026 furios
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <https://www.gnu.org/licenses/>.
 *
 * SPDX-License-Identifier: GPL-3.0-or-later
 */

use ashpd::desktop::location::{Accuracy, CreateSessionOptions, LocationProxy};
use futures_util::StreamExt;

async fn open_location_stream() -> ashpd::Result<(LocationProxy, impl futures_util::Stream<Item = ashpd::desktop::location::Location>)> {
    let proxy = LocationProxy::new().await?;

    let session = proxy
        .create_session(
            CreateSessionOptions::default()
                .set_accuracy(Accuracy::Exact)
                .set_distance_threshold(0u32)
                .set_time_threshold(1u32),
        )
        .await?;

    // Subscribe before start so we never miss the first fix.
    let stream = proxy.receive_location_updated().await?;

    proxy.start(&session, None, Default::default()).await?;

    Ok((proxy, stream))
}

/// Request a single location fix via the XDG Location portal.
/// Async — run inside a tokio runtime.
pub async fn get_location() -> Option<(f64, f64)> {
    match open_location_stream().await {
        Ok((_proxy, mut stream)) => stream.next().await.map(|l| (l.latitude(), l.longitude())),
        Err(e) => { eprintln!("[location] portal error: {e}"); None }
    }
}

/// Stream continuous location updates via the XDG Location portal.
/// Sends each fix as `(lat, lon, Option<heading_deg>)` over `tx` until the
/// receiver is dropped.  Heading is clockwise degrees from north (0 = north,
/// 90 = east) or `None` when the portal reports it as unavailable.
/// Async — run inside a tokio runtime.
pub async fn stream_location(tx: std::sync::mpsc::Sender<(f64, f64, Option<f64>)>) {
    match open_location_stream().await {
        Err(e) => eprintln!("[location] portal error: {e}"),
        Ok((_proxy, mut stream)) => {
            while let Some(location) = stream.next().await {
                eprintln!("[location] fix: {:.5}, {:.5}  acc={}m  heading={:?}",
                    location.latitude(), location.longitude(), location.accuracy(),
                    location.heading());
                if tx.send((location.latitude(), location.longitude(), location.heading())).is_err() {
                    break;
                }
            }
        }
    }
}
