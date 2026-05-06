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

use ashpd::desktop::location::{Accuracy, LocationProxy};
use futures_util::StreamExt;

/// Request a single location fix via the XDG Location portal (GeoClue2).
/// Async — run inside a tokio runtime.
pub async fn get_location() -> Option<(f64, f64)> {
    let proxy = LocationProxy::new().await.ok()?;
    let session = proxy
        .create_session(None, None, Some(Accuracy::Exact))
        .await
        .ok()?;

    // Subscribe to updates before calling start so we don't miss the first fix.
    let mut stream = proxy.receive_location_updated().await.ok()?;

    // start() asks the portal to begin location tracking; the first update
    // arrives on the stream above. Errors here are non-fatal (e.g. user denied).
    let _ = proxy.start(&session, None).await;

    stream.next().await.map(|l| (l.latitude(), l.longitude()))
}

/// Stream continuous location updates via the XDG Location portal.
/// Sends each fix as `(lat, lon)` over `tx` until the receiver is dropped.
/// Async — run inside a tokio runtime.
pub async fn stream_location(tx: std::sync::mpsc::Sender<(f64, f64)>) {
    let proxy = match LocationProxy::new().await {
        Ok(p) => p,
        Err(e) => { eprintln!("LocationProxy::new failed: {e}"); return; }
    };
    let session = match proxy.create_session(None, None, Some(Accuracy::Exact)).await {
        Ok(s) => s,
        Err(e) => { eprintln!("create_session failed: {e}"); return; }
    };
    let mut stream = match proxy.receive_location_updated().await {
        Ok(s) => s,
        Err(e) => { eprintln!("receive_location_updated failed: {e}"); return; }
    };
    let _ = proxy.start(&session, None).await;
    while let Some(location) = stream.next().await {
        if tx.send((location.latitude(), location.longitude())).is_err() {
            break; // receiver dropped — navigation stopped
        }
    }
}
