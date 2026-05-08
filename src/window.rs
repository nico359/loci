/* window.rs
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

use gtk::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib};
use shumate::prelude::*;
use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use crate::geocoding::PhotonFeature;

mod imp {
    use super::*;

    #[derive(Default, gtk::CompositeTemplate)]
    #[template(resource = "/io/github/nico359/loci/window.ui")]
    pub struct LociWindow {
        #[template_child]
        pub map: TemplateChild<shumate::SimpleMap>,
        #[template_child]
        pub search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        pub location_button: TemplateChild<gtk::Button>,
        #[template_child]
        pub nav_banner_revealer: TemplateChild<gtk::Revealer>,
        #[template_child]
        pub nav_instruction_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub nav_distance_label: TemplateChild<gtk::Label>,
        #[template_child]
        pub nav_maneuver_icon: TemplateChild<gtk::Image>,
        #[template_child]
        pub nav_stop_button: TemplateChild<gtk::Button>,

        pub marker_layer: RefCell<Option<shumate::MarkerLayer>>,
        pub location_layer: RefCell<Option<shumate::MarkerLayer>>,
        pub route_layer: RefCell<Option<shumate::PathLayer>>,
        pub current_location: RefCell<Option<(f64, f64)>>,
        pub current_results: RefCell<Vec<PhotonFeature>>,

        // Navigation state shared between the GPS thread and the main loop timer
        pub nav_controller: RefCell<Option<std::sync::Arc<ferrostar::navigation_controller::NavigationController>>>,
        pub nav_state: std::sync::Arc<std::sync::Mutex<Option<ferrostar::navigation_controller::models::NavState>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LociWindow {
        const NAME: &'static str = "LociWindow";
        type Type = super::LociWindow;
        type ParentType = adw::ApplicationWindow;

        fn class_init(klass: &mut Self::Class) {
            shumate::SimpleMap::ensure_type();
            klass.bind_template();
        }

        fn instance_init(obj: &glib::subclass::InitializingObject<Self>) {
            obj.init_template();
        }
    }

    impl ObjectImpl for LociWindow {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().setup_map();
            self.obj().setup_geocoding();
            self.obj().setup_location();
            self.obj().setup_navigation();
        }
    }

    impl WidgetImpl for LociWindow {}
    impl WindowImpl for LociWindow {}
    impl ApplicationWindowImpl for LociWindow {}
    impl AdwApplicationWindowImpl for LociWindow {}
}

glib::wrapper! {
    pub struct LociWindow(ObjectSubclass<imp::LociWindow>)
        @extends gtk::Widget, gtk::Window, gtk::ApplicationWindow, adw::ApplicationWindow,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl LociWindow {
    pub fn new<P: IsA<gtk::Application>>(application: &P) -> Self {
        glib::Object::builder()
            .property("application", application)
            .build()
    }

    fn setup_map(&self) {
        let imp = self.imp();

        if !shumate::VectorRenderer::is_supported() {
            eprintln!("ERROR: libshumate was compiled without vector tile support");
            return;
        }

        // Use the same tile server as GNOME Maps — static URL, OpenMapTiles v3 schema.
        // Glyphs from James Westman's font server (same as GNOME Maps).
        let style = serde_json::json!({
            "version": 8,
            "name": "Loci",
            "sources": {
                "vector-tiles": {
                    "type": "vector",
                    "tiles": ["https://tileserver.gnome.org/data/v3/{z}/{x}/{y}.pbf"],
                    "minzoom": 0,
                    "maxzoom": 14
                }
            },
            "glyphs": "https://tiles.maps.jwestman.net/fonts/{fontstack}/{range}.pbf",
            "layers": [
                {"id": "background", "type": "background",
                 "paint": {"background-color": "#f0ebe3"}},
                {"id": "water", "type": "fill",
                 "source": "vector-tiles", "source-layer": "water",
                 "paint": {"fill-color": "#a8d4f0"}},
                {"id": "landcover-park", "type": "fill",
                 "source": "vector-tiles", "source-layer": "landcover",
                 "filter": ["in", "class", "grass", "park", "forest"],
                 "paint": {"fill-color": "#c8e6c0"}},
                {"id": "road-minor", "type": "line",
                 "source": "vector-tiles", "source-layer": "transportation",
                 "filter": ["in", "class", "minor", "path", "service"],
                 "paint": {"line-color": "#d8d0c8", "line-width": 1}},
                {"id": "road-secondary", "type": "line",
                 "source": "vector-tiles", "source-layer": "transportation",
                 "filter": ["in", "class", "secondary", "tertiary"],
                 "paint": {"line-color": "#c0b8b0", "line-width": 2}},
                {"id": "road-primary", "type": "line",
                 "source": "vector-tiles", "source-layer": "transportation",
                 "filter": ["in", "class", "primary", "trunk", "motorway"],
                 "paint": {"line-color": "#e8c070", "line-width": 3}},
                {"id": "building", "type": "fill",
                 "source": "vector-tiles", "source-layer": "building",
                 "paint": {"fill-color": "#dbd5cc", "fill-outline-color": "#c8c0b8"}},
                {"id": "place-label", "type": "symbol",
                 "source": "vector-tiles", "source-layer": "place",
                 "layout": {"text-field": ["get", "name:latin"], "text-size": 13},
                 "paint": {"text-color": "#333", "text-halo-color": "#fff",
                           "text-halo-width": 1}}
            ]
        });

        match shumate::VectorRenderer::new("gnome-tiles", &style.to_string()) {
            Ok(renderer) => {
                imp.map.set_map_source(Some(&renderer));
                let viewport = imp.map.viewport().expect("SimpleMap has no viewport");
                viewport.set_zoom_level(12.0);
                viewport.set_location(52.5200, 13.4050);

                // Marker layers: one for search pins, one for current location
                let marker_layer = shumate::MarkerLayer::new(&viewport);
                imp.map.add_overlay_layer(&marker_layer);
                *imp.marker_layer.borrow_mut() = Some(marker_layer);

                let location_layer = shumate::MarkerLayer::new(&viewport);
                imp.map.add_overlay_layer(&location_layer);
                *imp.location_layer.borrow_mut() = Some(location_layer);

                // Path layer for route geometry (drawn below the marker layers)
                let route_layer = shumate::PathLayer::new(&viewport);
                route_layer.set_stroke_width(5.0);
                route_layer.set_stroke_color(Some(&gdk::RGBA::new(0.2, 0.5, 1.0, 0.9)));
                imp.map.add_overlay_layer(&route_layer);
                *imp.route_layer.borrow_mut() = Some(route_layer);
            }
            Err(e) => eprintln!("VectorRenderer::new error: {e}"),
        }
    }

    fn setup_geocoding(&self) {
        let imp = self.imp();

        // Results list inside a popover anchored to the search entry
        let list_box = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .build();
        list_box.add_css_class("boxed-list");

        let scrolled = gtk::ScrolledWindow::builder()
            .child(&list_box)
            .max_content_height(320)
            .propagate_natural_height(true)
            .min_content_width(300)
            .build();

        let popover = gtk::Popover::builder()
            .child(&scrolled)
            .has_arrow(false)
            .position(gtk::PositionType::Bottom)
            .autohide(true)
            .build();
        popover.set_parent(&imp.search_entry.get());

        // Channel: background search threads → main thread results
        let (tx, rx) = std::sync::mpsc::channel::<Vec<PhotonFeature>>();
        let rx = Arc::new(Mutex::new(rx));

        // On Enter: spawn search thread
        imp.search_entry.connect_activate({
            let tx = tx.clone();
            move |entry| {
                let query = entry.text().to_string();
                if query.is_empty() { return; }
                let tx = tx.clone();
                std::thread::spawn(move || {
                    let results = crate::geocoding::search(&query);
                    let _ = tx.send(results);
                });
            }
        });

        // Poll for results on the main loop; update popover when they arrive
        glib::timeout_add_local(std::time::Duration::from_millis(100), {
            let rx = rx.clone();
            let list_box = list_box.clone();
            let popover = popover.clone();
            let window_weak = self.downgrade();
            move || {
                if let Ok(results) = rx.lock().unwrap().try_recv() {
                    // Clear old rows
                    while let Some(child) = list_box.first_child() {
                        list_box.remove(&child);
                    }

                    if let Some(window) = window_weak.upgrade() {
                        *window.imp().current_results.borrow_mut() = results.clone();
                    }

                    if results.is_empty() {
                        popover.popdown();
                    } else {
                        for feat in &results {
                            let row = gtk::ListBoxRow::new();
                            let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                            hbox.set_margin_start(12);
                            hbox.set_margin_end(8);
                            hbox.set_margin_top(8);
                            hbox.set_margin_bottom(8);

                            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 2);
                            vbox.set_hexpand(true);

                            let name = gtk::Label::builder()
                                .label(&feat.name)
                                .xalign(0.0)
                                .ellipsize(gtk::pango::EllipsizeMode::End)
                                .build();

                            let subtitle = gtk::Label::builder()
                                .label(&feat.subtitle)
                                .xalign(0.0)
                                .ellipsize(gtk::pango::EllipsizeMode::End)
                                .build();
                            subtitle.add_css_class("caption");
                            subtitle.add_css_class("dim-label");

                            vbox.append(&name);
                            vbox.append(&subtitle);

                            // Navigate button
                            let nav_btn = gtk::Button::builder()
                                .icon_name("road-symbolic")
                                .tooltip_text("Navigate here")
                                .valign(gtk::Align::Center)
                                .build();
                            nav_btn.add_css_class("flat");
                            nav_btn.add_css_class("circular");

                            let dest = (feat.lat, feat.lon);
                            let nav_window_weak = window_weak.clone();
                            let nav_popover = popover.clone();
                            nav_btn.connect_clicked(move |_| {
                                nav_popover.popdown();
                                if let Some(w) = nav_window_weak.upgrade() {
                                    w.request_route(dest);
                                }
                            });

                            hbox.append(&vbox);
                            hbox.append(&nav_btn);
                            row.set_child(Some(&hbox));
                            list_box.append(&row);
                        }
                        popover.popup();
                    }
                }
                glib::ControlFlow::Continue
            }
        });

        // On row tap: pan map to location and place a marker
        list_box.connect_row_activated({
            let popover = popover.clone();
            let window_weak = self.downgrade();
            move |_, row| {
                let Some(window) = window_weak.upgrade() else { return };
                let imp = window.imp();
                let results = imp.current_results.borrow();
                let Some(feat) = results.get(row.index() as usize) else { return };
                let (lat, lon) = (feat.lat, feat.lon);
                drop(results);

                // Pan map
                let viewport = imp.map.viewport().expect("no viewport");
                viewport.set_zoom_level(14.0);
                viewport.set_location(lat, lon);

                // Replace marker
                if let Some(layer) = imp.marker_layer.borrow().as_ref() {
                    layer.remove_all();
                    let marker = shumate::Marker::new();
                    let img = gtk::Image::builder()
                        .icon_name("map-marker-symbolic")
                        .pixel_size(32)
                        .build();
                    img.add_css_class("accent");
                    marker.set_child(Some(&img));
                    marker.set_location(lat, lon);
                    layer.add_marker(&marker);
                }

                popover.popdown();
            }
        });
    }

    /// Request a route from the current location to `destination`, draw it, and start navigation.
    fn request_route(&self, destination: (f64, f64)) {
        let imp = self.imp();
        let origin = match *imp.current_location.borrow() {
            Some(loc) => loc,
            None => {
                eprintln!("No location fix yet — press the location button first");
                return;
            }
        };

        let (tx, rx) = std::sync::mpsc::channel::<ferrostar::models::Route>();
        let rx = Arc::new(Mutex::new(rx));

        // Fetch route in background thread
        std::thread::spawn(move || {
            match crate::routing::get_route(origin, destination, crate::routing::DEFAULT_VALHALLA_URL, "auto") {
                Some(route) => { let _ = tx.send(route); }
                None => eprintln!("Routing request failed"),
            }
        });

        // When route arrives: draw it + start navigation
        glib::timeout_add_local(std::time::Duration::from_millis(200), {
            let window_weak = self.downgrade();
            move || {
                if let Ok(route) = rx.lock().unwrap().try_recv() {
                    let Some(window) = window_weak.upgrade() else {
                        return glib::ControlFlow::Break;
                    };
                    // Draw the geometry
                    let imp = window.imp();
                    let layer_opt = imp.route_layer.borrow().clone();
                    if let Some(layer) = layer_opt {
                        layer.remove_all();
                        for c in &route.geometry {
                            let node = shumate::Coordinate::new_full(c.lat, c.lng);
                            layer.add_node(&node);
                        }
                    };
                    // Start navigation
                    window.start_navigation(origin, route);
                    return glib::ControlFlow::Break;
                }
                glib::ControlFlow::Continue
            }
        });
    }

    /// Start the Ferrostar NavigationController and begin streaming GPS updates.
    fn start_navigation(&self, origin: (f64, f64), route: ferrostar::models::Route) {
        use ferrostar::navigation_controller::NavigationController;
        use ferrostar::navigation_controller::Navigator;
        use ferrostar::navigation_controller::models::{
            NavigationControllerConfig, WaypointAdvanceMode, CourseFiltering,
        };
        use ferrostar::navigation_controller::step_advance::conditions::DistanceToEndOfStepCondition;
        use ferrostar::deviation_detection::RouteDeviationTracking;
        use ferrostar::models::{GeographicCoordinate, UserLocation};
        use std::sync::Arc;

        let step_advance = Arc::new(DistanceToEndOfStepCondition {
            distance: 25,
            minimum_horizontal_accuracy: 50,
        });
        let config = NavigationControllerConfig {
            waypoint_advance: WaypointAdvanceMode::WaypointWithinRange(50.0),
            step_advance_condition: step_advance.clone(),
            arrival_step_advance_condition: Arc::new(DistanceToEndOfStepCondition {
                distance: 10,
                minimum_horizontal_accuracy: 50,
            }),
            route_deviation_tracking: RouteDeviationTracking::None,
            snapped_location_course_filtering: CourseFiltering::Raw,
        };

        let controller = Arc::new(NavigationController::new(route, config));
        eprintln!("[nav] NavigationController created, calling get_initial_state...");

        let initial_location = UserLocation {
            coordinates: GeographicCoordinate { lat: origin.0, lng: origin.1 },
            horizontal_accuracy: 10.0,
            course_over_ground: None,
            timestamp: std::time::SystemTime::now(),
            speed: None,
        };
        // get_initial_state() returns a NavState already in TripState::Navigating
        let first_state = controller.get_initial_state(initial_location);

        let imp = self.imp();
        *imp.nav_controller.borrow_mut() = Some(controller.clone());

        // Store first state
        {
            let mut state_lock = imp.nav_state.lock().unwrap();
            *state_lock = Some(first_state.clone());
        }

        // Show nav banner and populate with first instruction
        imp.nav_banner_revealer.set_reveal_child(true);
        Self::update_nav_banner(imp, &first_state);

        // Spawn GPS streaming thread — feeds location into the navigation controller
        let nav_state = imp.nav_state.clone();
        let (loc_tx, loc_rx) = std::sync::mpsc::channel::<(f64, f64)>();
        let loc_rx = Arc::new(Mutex::new(loc_rx));

        // GPS thread
        std::thread::spawn(move || {
            let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
            rt.block_on(crate::location::stream_location(loc_tx));
        });

        // Navigation update loop — runs on main thread
        glib::timeout_add_local(std::time::Duration::from_millis(500), {
            let window_weak = self.downgrade();
            let controller = controller.clone();
            let nav_state = nav_state.clone();
            let loc_rx = loc_rx.clone();
            move || {
                // Drain all pending location fixes (use the latest one)
                let mut latest: Option<(f64, f64)> = None;
                while let Ok(fix) = loc_rx.lock().unwrap().try_recv() {
                    latest = Some(fix);
                }

                let Some((lat, lon)) = latest else {
                    return glib::ControlFlow::Continue;
                };

                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };

                use ferrostar::navigation_controller::Navigator;
                use ferrostar::models::{GeographicCoordinate, UserLocation};

                let user_location = UserLocation {
                    coordinates: GeographicCoordinate { lat, lng: lon },
                    horizontal_accuracy: 10.0,
                    course_over_ground: None,
                    timestamp: std::time::SystemTime::now(),
                    speed: None,
                };

                // Update navigation state
                let new_state = {
                    let mut state_lock = nav_state.lock().unwrap();
                    let old_state = state_lock.take().unwrap();
                    let updated = controller.update_user_location(user_location, old_state);
                    *state_lock = Some(updated.clone());
                    updated
                };

                let imp = window.imp();

                // Update location dot
                *imp.current_location.borrow_mut() = Some((lat, lon));
                let loc_layer = imp.location_layer.borrow().clone();
                if let Some(layer) = loc_layer {
                    layer.remove_all();
                    let marker = shumate::Marker::new();
                    let dot = gtk::Box::builder().width_request(18).height_request(18).build();
                    dot.add_css_class("location-dot");
                    marker.set_child(Some(&dot));
                    marker.set_location(lat, lon);
                    layer.add_marker(&marker);
                };

                // Pan map to follow user
                if let Some(viewport) = imp.map.viewport() {
                    viewport.set_location(lat, lon);
                }

                // Update banner from trip state
                use ferrostar::navigation_controller::models::TripState;
                if matches!(new_state.trip_state(), TripState::Complete { .. }) {
                    imp.nav_instruction_label.set_text("You have arrived!");
                    imp.nav_distance_label.set_text("");
                    imp.nav_maneuver_icon.set_icon_name(Some("flag-filled-symbolic"));
                    glib::timeout_add_local_once(std::time::Duration::from_secs(3), {
                        let window_weak = window.downgrade();
                        move || {
                            if let Some(w) = window_weak.upgrade() {
                                w.stop_navigation();
                            }
                        }
                    });
                    return glib::ControlFlow::Break;
                }
                Self::update_nav_banner(imp, &new_state);

                glib::ControlFlow::Continue
            }
        });
    }

    fn setup_navigation(&self) {
        let imp = self.imp();
        imp.nav_stop_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                if let Some(w) = window_weak.upgrade() {
                    w.stop_navigation();
                }
            }
        });
    }

    fn stop_navigation(&self) {
        let imp = self.imp();
        imp.nav_banner_revealer.set_reveal_child(false);
        *imp.nav_controller.borrow_mut() = None;
        *imp.nav_state.lock().unwrap() = None;
        // Clear route layer
        let layer_opt = imp.route_layer.borrow().clone();
        if let Some(layer) = layer_opt {
            layer.remove_all();
        };
    }

    /// Update the nav banner labels/icon from a NavState.
    fn update_nav_banner(imp: &imp::LociWindow, state: &ferrostar::navigation_controller::models::NavState) {
        use ferrostar::navigation_controller::models::TripState;
        eprintln!("[nav] update_nav_banner called, trip_state variant check...");
        match state.trip_state() {
            TripState::Navigating { visual_instruction, remaining_steps, progress, .. } => {
                eprintln!("[nav] TripState::Navigating — dist_to_maneuver={:.0}m", progress.distance_to_next_maneuver);
                let text = visual_instruction
                    .as_ref()
                    .map(|vi| vi.primary_content.text.as_str())
                    .or_else(|| remaining_steps.first().and_then(|s| s.road_name.as_deref()))
                    .unwrap_or("Continue")
                    .to_string();
                let icon = maneuver_icon(
                    visual_instruction.as_ref().and_then(|vi| vi.primary_content.maneuver_type),
                    visual_instruction.as_ref().and_then(|vi| vi.primary_content.maneuver_modifier),
                );
                imp.nav_instruction_label.set_text(&text);
                imp.nav_distance_label.set_text(&format_distance(progress.distance_to_next_maneuver));
                imp.nav_maneuver_icon.set_icon_name(Some(icon));
            }
            TripState::Complete { .. } => eprintln!("[nav] TripState::Complete"),
            TripState::Idle { .. } => eprintln!("[nav] TripState::Idle — nav controller did not advance"),
        }
    }

    fn setup_location(&self) {
        let imp = self.imp();

        let (tx, rx) = std::sync::mpsc::channel::<(f64, f64)>();
        let rx = Arc::new(Mutex::new(rx));

        // Start streaming location continuously in the background.
        // This keeps the GeoClue session alive so GPS can improve over time.
        {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("tokio runtime");
                rt.block_on(crate::location::stream_location(tx));
            });
        }

        // Location button: just re-centres the map on the latest known fix.
        // No longer starts a new session — the stream is already running.
        imp.location_button.connect_clicked({
            let window_weak = self.downgrade();
            move |_| {
                let Some(window) = window_weak.upgrade() else { return; };
                let imp = window.imp();
                let loc = imp.current_location.borrow().clone();
                if let Some((lat, lon)) = loc {
                    if let Some(viewport) = imp.map.viewport() {
                        viewport.set_zoom_level(15.0);
                        viewport.set_location(lat, lon);
                    }
                }
            }
        });

        // Poll for location fixes on the main loop and update the blue dot.
        // Only pan the map on the very first fix; after that the button does it.
        let first_fix_received = std::cell::Cell::new(false);
        glib::timeout_add_local(std::time::Duration::from_millis(500), {
            let window_weak = self.downgrade();
            move || {
                // Drain to latest fix only
                let mut latest = None;
                while let Ok(fix) = rx.lock().unwrap().try_recv() {
                    latest = Some(fix);
                }
                let Some((lat, lon)) = latest else {
                    return glib::ControlFlow::Continue;
                };
                let Some(window) = window_weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                let imp = window.imp();

                // Store fix so routing and location button can use it
                *imp.current_location.borrow_mut() = Some((lat, lon));

                // Pan to first fix automatically; subsequent pans only on button press
                if !first_fix_received.get() {
                    first_fix_received.set(true);
                    if let Some(viewport) = imp.map.viewport() {
                        viewport.set_zoom_level(15.0);
                        viewport.set_location(lat, lon);
                    }
                }

                // Update blue dot
                let layer_opt = imp.location_layer.borrow().clone();
                if let Some(layer) = layer_opt {
                    layer.remove_all();
                    let marker = shumate::Marker::new();
                    let dot = gtk::Box::builder()
                        .width_request(18)
                        .height_request(18)
                        .build();
                    dot.add_css_class("location-dot");
                    marker.set_child(Some(&dot));
                    marker.set_location(lat, lon);
                    layer.add_marker(&marker);
                }
                glib::ControlFlow::Continue
            }
        });
    }
}

fn maneuver_icon(
    maneuver_type: Option<ferrostar::models::ManeuverType>,
    maneuver_modifier: Option<ferrostar::models::ManeuverModifier>,
) -> &'static str {
    use ferrostar::models::{ManeuverModifier as Mod, ManeuverType as Typ};
    match (maneuver_type, maneuver_modifier) {
        (Some(Typ::Arrive), _) => "flag-filled-symbolic",
        (Some(Typ::Depart), _) => "find-location-symbolic",
        (Some(Typ::Roundabout) | Some(Typ::Rotary) | Some(Typ::ExitRoundabout) | Some(Typ::ExitRotary), _) => "arrow-circular-symbolic",
        (_, Some(Mod::Left) | Some(Mod::SharpLeft) | Some(Mod::SlightLeft)) => "go-previous-symbolic",
        (_, Some(Mod::Right) | Some(Mod::SharpRight) | Some(Mod::SlightRight)) => "go-next-symbolic",
        (_, Some(Mod::UTurn)) => "arrow-back-symbolic",
        _ => "go-up-symbolic",
    }
}

fn format_distance(meters: f64) -> String {
    if meters >= 1000.0 {
        format!("{:.1} km", meters / 1000.0)
    } else {
        format!("{} m", meters.round() as u32)
    }
}
