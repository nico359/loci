/* util.rs
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

//! Small, dependency-free helpers: geodesic maths and human-readable formatting.

/// Haversine distance in metres between two WGS-84 coordinates.
pub(crate) fn haversine(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0;
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R * a.sqrt().asin()
}

/// Great-circle bearing from point 1 → point 2, in clockwise degrees from north [0, 360).
pub(crate) fn compute_bearing(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let lat1r = lat1.to_radians();
    let lat2r = lat2.to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let y = dlon.sin() * lat2r.cos();
    let x = lat1r.cos() * lat2r.sin() - lat1r.sin() * lat2r.cos() * dlon.cos();
    let bearing = y.atan2(x).to_degrees();
    (bearing + 360.0) % 360.0
}

/// Capitalises the first letter of `s`, replacing underscores with spaces.
pub(crate) fn capitalize(s: &str) -> String {
    let spaced = s.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().to_string() + chars.as_str(),
    }
}

/// Formats a distance in metres as metres (below 1 km) or kilometres.
pub(crate) fn format_distance(meters: f64) -> String {
    if meters >= 1000.0 {
        format!("{:.1} km", meters / 1000.0)
    } else {
        format!("{} m", meters.round() as u32)
    }
}

/// Formats a duration in seconds as minutes, or hours and minutes.
pub(crate) fn format_duration(seconds: f64) -> String {
    let total = seconds.round() as u64;
    let hours = total / 3600;
    let mins = (total % 3600) / 60;
    if hours > 0 {
        format!("{} h {} min", hours, mins)
    } else if mins == 0 {
        "< 1 min".to_string()
    } else {
        format!("{} min", mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_one_degree_of_latitude() {
        // 1° of latitude is ~111.19 km on a sphere of radius 6371 km.
        let d = haversine(0.0, 0.0, 1.0, 0.0);
        assert!((d - 111_194.9).abs() < 1.0, "got {d}");
    }

    #[test]
    fn haversine_is_symmetric_and_zero_for_same_point() {
        assert_eq!(haversine(52.5, 13.4, 52.5, 13.4), 0.0);
        let a = haversine(52.52, 13.405, 48.13, 11.57);
        let b = haversine(48.13, 11.57, 52.52, 13.405);
        assert!((a - b).abs() < 1e-6, "not symmetric: {a} vs {b}");
    }

    #[test]
    fn bearing_to_cardinal_directions() {
        assert!(compute_bearing(0.0, 0.0, 1.0, 0.0).abs() < 1e-6); // north
        assert!((compute_bearing(0.0, 0.0, 0.0, 1.0) - 90.0).abs() < 1e-6); // east
    }

    #[test]
    fn distance_formatting() {
        assert_eq!(format_distance(0.4), "0 m");
        assert_eq!(format_distance(500.0), "500 m");
        assert_eq!(format_distance(1499.0), "1.5 km");
    }

    #[test]
    fn duration_formatting() {
        assert_eq!(format_duration(30.0), "< 1 min");
        assert_eq!(format_duration(90.0), "1 min");
        assert_eq!(format_duration(3661.0), "1 h 1 min");
    }

    #[test]
    fn capitalize_replaces_underscores() {
        assert_eq!(capitalize("restaurant"), "Restaurant");
        assert_eq!(capitalize("fast_food"), "Fast food");
        assert_eq!(capitalize(""), "");
    }
}
