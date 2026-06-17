/* routing.rs
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

use ferrostar::routing_adapters::osrm::OsrmResponseParser;
use ferrostar::routing_adapters::RouteResponseParser;
use ferrostar::models::{BoundingBox, GeographicCoordinate, ManeuverModifier, ManeuverType, Route, RouteStep, VisualInstruction, VisualInstructionContent, Waypoint, WaypointKind};

/// Base URL for the OSRM routing service.
pub const DEFAULT_OSRM_BASE_URL: &str = "https://routing.openstreetmap.de";

/// Request a route from `origin` to `destination` using OSRM.
///
/// `origin` and `destination` are `(lat, lon)` pairs.
/// The `profile` is the OSRM vehicle profile: `"car"`, `"bike"`, or `"foot"`.
/// Returns the first Route on success.
pub fn get_route(
    origin: (f64, f64),
    destination: (f64, f64),
    osrm_base_url: &str,
    profile: &str,
) -> Option<Route> {
    // OSRM expects coordinates in lon,lat order.
    let url = format!(
        "{}/routed-{}/route/v1/driving/{},{};{},{}?overview=full&steps=true&annotations=duration,distance",
        osrm_base_url.trim_end_matches('/'),
        profile,
        origin.1, origin.0,
        destination.1, destination.0,
    );

    eprintln!("[routing] GET {url}");

    let client = reqwest::blocking::Client::new();
    let bytes = match client.get(&url).send() {
        Ok(resp) => {
            let status = resp.status();
            let bytes = resp.bytes().ok()?;
            if !status.is_success() {
                eprintln!("[routing] OSRM error {status}: {}", String::from_utf8_lossy(&bytes));
                return None;
            }
            bytes
        }
        Err(e) => {
            eprintln!("[routing] HTTP error: {e}");
            return None;
        }
    };

    eprintln!("[routing] response {} bytes", bytes.len());

    // Standard OSRM uses polyline precision 5.
    let mut routes = match OsrmResponseParser::new(5).parse_response(bytes.to_vec()) {
        Ok(routes) => {
            eprintln!("[routing] parsed {} route(s)", routes.len());
            routes
        }
        Err(e) => {
            eprintln!("[routing] parse error: {e:?}");
            return None;
        }
    };

    // Standard OSRM doesn't include pre-synthesized instruction text or banner instructions.
    // Parse the raw JSON to extract maneuver type/modifier/road-name and synthesize them.
    if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        let raw_steps: Vec<serde_json::Value> = json["routes"]
            .as_array()
            .and_then(|r| r.first())
            .and_then(|r| r["legs"].as_array())
            .map(|legs| {
                legs.iter()
                    .flat_map(|leg| leg["steps"].as_array().cloned().unwrap_or_default())
                    .collect()
            })
            .unwrap_or_default();

        if let Some(route) = routes.first_mut() {
            for (i, (step, raw)) in route.steps.iter_mut().zip(raw_steps.iter()).enumerate() {
                let maneuver = &raw["maneuver"];
                let mtype_str = maneuver["type"].as_str().unwrap_or("");
                let mmod_str = maneuver["modifier"].as_str().unwrap_or("");
                let road_name = raw["name"].as_str().unwrap_or("").trim();
                let exit_num = maneuver["exit"].as_u64();

                let mtype = parse_maneuver_type(mtype_str);
                let mmod = parse_maneuver_modifier(mmod_str);
                // step.instruction is the maneuver that started this step (already done);
                // it serves as the fallback label for the current road.
                step.instruction = synthesize_instruction(mtype, mmod, road_name, exit_num);

                // Visual instruction: preview the NEXT step's maneuver, triggered near
                // the end of the current step.  In OSRM format the maneuver is at the
                // *start* of each step, so the action the user needs to prepare for
                // while travelling step[i] is the maneuver that opens step[i+1].
                if let Some(next_raw) = raw_steps.get(i + 1) {
                    let nm = &next_raw["maneuver"];
                    let next_mtype = parse_maneuver_type(nm["type"].as_str().unwrap_or(""));
                    let next_mmod  = parse_maneuver_modifier(nm["modifier"].as_str().unwrap_or(""));
                    let next_road  = next_raw["name"].as_str().unwrap_or("").trim();
                    let next_exit  = nm["exit"].as_u64();
                    let next_text  = synthesize_instruction(next_mtype, next_mmod, next_road, next_exit);

                    let trigger = (step.distance * 0.3).clamp(30.0, 250.0);
                    step.visual_instructions = vec![VisualInstruction {
                        primary_content: VisualInstructionContent {
                            text: next_text,
                            maneuver_type: next_mtype,
                            maneuver_modifier: next_mmod,
                            roundabout_exit_degrees: None,
                            lane_info: None,
                            exit_numbers: vec![],
                        },
                        secondary_content: None,
                        sub_content: None,
                        trigger_distance_before_maneuver: trigger,
                    }];
                }
                // Last step (arrive) needs no visual instruction.
            }
        }
    }

    routes.into_iter().next()
}

fn parse_maneuver_type(s: &str) -> Option<ManeuverType> {
    match s {
        "turn" => Some(ManeuverType::Turn),
        "new name" => Some(ManeuverType::NewName),
        "depart" => Some(ManeuverType::Depart),
        "arrive" => Some(ManeuverType::Arrive),
        "merge" => Some(ManeuverType::Merge),
        "on ramp" => Some(ManeuverType::OnRamp),
        "off ramp" => Some(ManeuverType::OffRamp),
        "fork" => Some(ManeuverType::Fork),
        "end of road" => Some(ManeuverType::EndOfRoad),
        "continue" => Some(ManeuverType::Continue),
        "roundabout" => Some(ManeuverType::Roundabout),
        "rotary" => Some(ManeuverType::Rotary),
        "roundabout turn" => Some(ManeuverType::RoundaboutTurn),
        "notification" => Some(ManeuverType::Notification),
        "exit roundabout" => Some(ManeuverType::ExitRoundabout),
        "exit rotary" => Some(ManeuverType::ExitRotary),
        _ => None,
    }
}

fn parse_maneuver_modifier(s: &str) -> Option<ManeuverModifier> {
    match s {
        "uturn" => Some(ManeuverModifier::UTurn),
        "sharp right" => Some(ManeuverModifier::SharpRight),
        "right" => Some(ManeuverModifier::Right),
        "slight right" => Some(ManeuverModifier::SlightRight),
        "straight" => Some(ManeuverModifier::Straight),
        "slight left" => Some(ManeuverModifier::SlightLeft),
        "left" => Some(ManeuverModifier::Left),
        "sharp left" => Some(ManeuverModifier::SharpLeft),
        _ => None,
    }
}

fn modifier_text(mmod: Option<ManeuverModifier>) -> &'static str {
    match mmod {
        Some(ManeuverModifier::UTurn) => "make a U-turn",
        Some(ManeuverModifier::SharpRight) => "turn sharp right",
        Some(ManeuverModifier::Right) => "turn right",
        Some(ManeuverModifier::SlightRight) => "keep right",
        Some(ManeuverModifier::Straight) | None => "continue straight",
        Some(ManeuverModifier::SlightLeft) => "keep left",
        Some(ManeuverModifier::Left) => "turn left",
        Some(ManeuverModifier::SharpLeft) => "turn sharp left",
    }
}

fn synthesize_instruction(
    mtype: Option<ManeuverType>,
    mmod: Option<ManeuverModifier>,
    road_name: &str,
    exit_num: Option<u64>,
) -> String {
    let onto = if road_name.is_empty() {
        String::new()
    } else {
        format!(" onto {road_name}")
    };
    let on = if road_name.is_empty() {
        String::new()
    } else {
        format!(" on {road_name}")
    };

    match mtype {
        Some(ManeuverType::Depart) => {
            if road_name.is_empty() { "Depart".into() }
            else { format!("Head {}{on}", modifier_text(mmod).trim_start_matches("continue ").trim_start_matches("turn ")) }
        }
        Some(ManeuverType::Arrive) => "You have arrived at your destination".into(),
        Some(ManeuverType::Turn) | Some(ManeuverType::EndOfRoad) => {
            let dir = modifier_text(mmod);
            format!("{}{onto}", capitalize(dir))
        }
        Some(ManeuverType::NewName) => {
            if road_name.is_empty() { "Continue".into() }
            else { format!("Continue{onto}") }
        }
        Some(ManeuverType::Continue) | Some(ManeuverType::Notification) => {
            format!("Continue{on}")
        }
        Some(ManeuverType::Merge) => {
            format!("Merge{onto}")
        }
        Some(ManeuverType::OnRamp) => {
            let dir = modifier_text(mmod);
            format!("Take the ramp on the {}", dir.trim_start_matches("turn ").trim_start_matches("keep "))
        }
        Some(ManeuverType::OffRamp) => {
            format!("Take the exit{onto}")
        }
        Some(ManeuverType::Fork) => {
            let side = match mmod {
                Some(ManeuverModifier::Left) | Some(ManeuverModifier::SlightLeft) | Some(ManeuverModifier::SharpLeft) => "left",
                _ => "right",
            };
            format!("Keep {side} at the fork{onto}")
        }
        Some(ManeuverType::Roundabout) | Some(ManeuverType::Rotary) => {
            if let Some(n) = exit_num {
                format!("Enter the roundabout and take the {n}{} exit", ordinal_suffix(n))
            } else {
                "Enter the roundabout".into()
            }
        }
        Some(ManeuverType::RoundaboutTurn) => {
            let dir = modifier_text(mmod);
            format!("{} on the roundabout{onto}", capitalize(dir))
        }
        Some(ManeuverType::ExitRoundabout) | Some(ManeuverType::ExitRotary) => {
            format!("Exit the roundabout{onto}")
        }
        None => {
            if road_name.is_empty() { "Continue".into() }
            else { format!("Continue{on}") }
        }
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

/// Route using OSM Scout Server's Valhalla endpoint (`/v2/route`).
/// Blocking — call from a background thread.
pub fn get_route_valhalla(
    origin: (f64, f64),
    destination: (f64, f64),
    profile: &str,
) -> Option<Route> {
    // Map loci profile names to Valhalla costing models.
    let costing = match profile {
        "bike" => "bicycle",
        "foot" => "pedestrian",
        _ => "auto",
    };

    let body = serde_json::json!({
        "locations": [
            {"lat": origin.0, "lon": origin.1, "type": "break"},
            {"lat": destination.0, "lon": destination.1, "type": "break"}
        ],
        "costing": costing,
        "units": "km"
    });

    eprintln!("[routing/valhalla] POST http://localhost:8553/v2/route costing={costing}");

    let client = reqwest::blocking::Client::new();
    let resp = match client
        .get("http://localhost:8553/v2/route")
        .query(&[("json", body.to_string())])
        .send()
    {
        Ok(r) => r,
        Err(e) => { eprintln!("[routing/valhalla] HTTP error: {e}"); return None; }
    };

    let status = resp.status();
    let bytes = resp.bytes().ok()?;
    if !status.is_success() {
        eprintln!("[routing/valhalla] error {status}: {}", String::from_utf8_lossy(&bytes));
        return None;
    }

    parse_valhalla_response(&bytes)
}

/// Dispatcher: selects OSRM or Valhalla based on profile.
pub fn get_route_with_source(
    origin: (f64, f64),
    destination: (f64, f64),
    map_profile: &str,
    route_profile: &str,
) -> Option<Route> {
    if map_profile == "offline" {
        get_route_valhalla(origin, destination, route_profile)
    } else {
        get_route(origin, destination, DEFAULT_OSRM_BASE_URL, route_profile)
    }
}

fn parse_valhalla_response(bytes: &[u8]) -> Option<Route> {
    let json: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let trip = &json["trip"];

    // Valhalla uses polyline6 for its encoded shape.
    let decode_shape = |encoded: &str| -> Vec<GeographicCoordinate> {
        polyline::decode_polyline(encoded, 6)
            .unwrap_or_else(|_| geo::LineString::new(vec![]))
            .into_iter()
            .map(|c| GeographicCoordinate { lat: c.y, lng: c.x })
            .collect()
    };

    // Collect all geometry from all legs.
    let legs = trip["legs"].as_array()?;
    let mut all_geometry: Vec<GeographicCoordinate> = Vec::new();
    let mut all_steps: Vec<RouteStep> = Vec::new();
    let mut total_distance_m = 0.0_f64;

    for leg in legs {
        let leg_shape = leg["shape"].as_str().unwrap_or("");
        let leg_geom = decode_shape(leg_shape);

        let maneuvers = leg["maneuvers"].as_array().cloned().unwrap_or_default();
        for (i, m) in maneuvers.iter().enumerate() {
            let begin_idx = m["begin_shape_index"].as_u64().unwrap_or(0) as usize;
            let end_idx = m["end_shape_index"].as_u64()
                .unwrap_or_else(|| (leg_geom.len() as u64).saturating_sub(1)) as usize;

            let step_geom: Vec<GeographicCoordinate> = leg_geom
                .get(begin_idx..=end_idx.min(leg_geom.len().saturating_sub(1)))
                .unwrap_or(&[])
                .to_vec();

            // Valhalla gives length in km (with "units":"km").
            let dist_m = m["length"].as_f64().unwrap_or(0.0) * 1000.0;
            let duration_s = m["time"].as_f64().unwrap_or(0.0);
            total_distance_m += dist_m;

            let road_name = m["street_names"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(str::to_owned);

            let instruction_text = m["instruction"].as_str().unwrap_or("").to_owned();

            // Preview the NEXT step's maneuver, triggered near the end of this step —
            // same pattern as the OSRM fix: maneuver[i] describes what happens at the
            // START of step[i], so while travelling step[i] the user needs to prepare
            // for maneuver[i+1].
            let next_m = maneuvers.get(i + 1);
            let visual_instructions = if let Some(nm) = next_m {
                let next_text = nm["instruction"].as_str().unwrap_or("").to_owned();
                let next_mtype = valhalla_maneuver_type(nm["type"].as_u64().unwrap_or(0));
                let next_mmod = valhalla_maneuver_modifier(nm["type"].as_u64().unwrap_or(0));
                let trigger = (dist_m * 0.3).clamp(30.0, 250.0);
                vec![VisualInstruction {
                    primary_content: VisualInstructionContent {
                        text: next_text,
                        maneuver_type: next_mtype,
                        maneuver_modifier: next_mmod,
                        roundabout_exit_degrees: None,
                        exit_numbers: vec![],
                        lane_info: None,
                    },
                    secondary_content: None,
                    sub_content: None,
                    trigger_distance_before_maneuver: trigger,
                }]
            } else {
                vec![]
            };

            all_steps.push(RouteStep {
                geometry: step_geom.clone(),
                distance: dist_m,
                duration: duration_s,
                road_name,
                instruction: instruction_text,
                visual_instructions,
                spoken_instructions: vec![],
                annotations: None,
                incidents: vec![],
                exits: vec![],
                driving_side: None,
                roundabout_exit_number: None,
            });

            // Append step geometry to whole-route geometry (avoid duplicate points).
            if all_geometry.is_empty() {
                all_geometry.extend_from_slice(&step_geom);
            } else if let Some(first) = step_geom.first() {
                let last = all_geometry.last().unwrap();
                let skip = (last.lat == first.lat && last.lng == first.lng) as usize;
                all_geometry.extend_from_slice(&step_geom[skip..]);
            }
        }
    }

    if all_geometry.is_empty() { return None; }

    let (min_lat, max_lat, min_lng, max_lng) = all_geometry.iter().fold(
        (f64::MAX, f64::MIN, f64::MAX, f64::MIN),
        |(mnlat, mxlat, mnlng, mxlng), c| {
            (mnlat.min(c.lat), mxlat.max(c.lat), mnlng.min(c.lng), mxlng.max(c.lng))
        },
    );

    let waypoints = vec![
        Waypoint { coordinate: all_geometry.first().copied().unwrap(), kind: WaypointKind::Break, properties: None },
        Waypoint { coordinate: all_geometry.last().copied().unwrap(), kind: WaypointKind::Break, properties: None },
    ];

    Some(Route {
        geometry: all_geometry,
        bbox: BoundingBox {
            sw: GeographicCoordinate { lat: min_lat, lng: min_lng },
            ne: GeographicCoordinate { lat: max_lat, lng: max_lng },
        },
        distance: total_distance_m,
        waypoints,
        steps: all_steps,
    })
}

/// Convert a Valhalla maneuver type integer to a ferrostar ManeuverType.
/// See https://valhalla.github.io/valhalla/api/turn-by-turn/api-reference/#maneuver-types
fn valhalla_maneuver_type(t: u64) -> Option<ManeuverType> {
    match t {
        1 | 2 | 3 => Some(ManeuverType::Depart),
        4 | 5 | 6 => Some(ManeuverType::Arrive),
        7 | 8 | 17 | 22 => Some(ManeuverType::Continue),
        9 | 10 | 11 | 14 | 15 | 16 => Some(ManeuverType::Turn),
        18 | 19 | 20 | 21 => Some(ManeuverType::OffRamp),
        23 | 24 => Some(ManeuverType::Fork),
        25 => Some(ManeuverType::Merge),
        26 => Some(ManeuverType::Roundabout),
        27 => Some(ManeuverType::ExitRoundabout),
        _ => None,
    }
}

/// Map Valhalla maneuver type integer to a ManeuverModifier.
/// Numbers from Valhalla's DirectionsLeg_Maneuver_Type protobuf enum.
fn valhalla_maneuver_modifier(t: u64) -> Option<ManeuverModifier> {
    match t {
        9  => Some(ManeuverModifier::SlightRight),
        10 => Some(ManeuverModifier::Right),
        11 => Some(ManeuverModifier::SharpRight),
        12 | 13 => Some(ManeuverModifier::UTurn),
        14 => Some(ManeuverModifier::SharpLeft),
        15 => Some(ManeuverModifier::Left),
        16 => Some(ManeuverModifier::SlightLeft),
        _ => None,
    }
}

fn ordinal_suffix(n: u64) -> &'static str {
    match n % 10 {
        1 if n % 100 != 11 => "st",
        2 if n % 100 != 12 => "nd",
        3 if n % 100 != 13 => "rd",
        _ => "th",
    }
}
