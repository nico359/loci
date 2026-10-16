/* history.rs
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

//! Recently selected places, persisted to the user data directory.
//!
//! Shown when a search field is focused but still empty, so a place that was
//! picked before can be reused without retyping the query.

use gtk::glib;

use crate::geocoding::PhotonFeature;
use crate::util::haversine;

/// Maximum number of entries kept.
const MAX_ENTRIES: usize = 10;

fn path() -> std::path::PathBuf {
    let mut path = glib::user_data_dir();
    path.push("loci");
    path.push("search-history.json");
    path
}

/// The recently selected places, newest first.
pub(crate) fn load() -> Vec<PhotonFeature> {
    let Ok(raw) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Record a selected place. Entries describing the same place (same name
/// within 100 m) are replaced, and the list is capped at [`MAX_ENTRIES`].
pub(crate) fn record(feature: &PhotonFeature) {
    let mut entries = load();
    entries.retain(|existing| {
        !(existing.name == feature.name
            && haversine(existing.lat, existing.lon, feature.lat, feature.lon) < 100.0)
    });
    entries.insert(0, feature.clone());
    entries.truncate(MAX_ENTRIES);

    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(&entries) {
        let _ = std::fs::write(path, json);
    }
}
