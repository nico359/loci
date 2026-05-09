/* application.rs
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

use gettextrs::gettext;
use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{gdk, gio, glib};

use crate::config::VERSION;
use crate::LociWindow;

mod imp {
    use super::*;

    #[derive(Debug, Default)]
    pub struct LociApplication {}

    #[glib::object_subclass]
    impl ObjectSubclass for LociApplication {
        const NAME: &'static str = "LociApplication";
        type Type = super::LociApplication;
        type ParentType = adw::Application;
    }

    impl ObjectImpl for LociApplication {
        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.setup_gactions();
            obj.set_accels_for_action("app.quit", &["<control>q"]);
        }
    }

    impl ApplicationImpl for LociApplication {
        fn startup(&self) {
            self.parent_startup();
            // Register bundled icons so GTK can find them by name
            if let Some(display) = gdk::Display::default() {
                gtk::IconTheme::for_display(&display)
                    .add_resource_path("/io/github/nico359/loci/icons");
            }
            // Load stylesheet after GTK is initialized
            let provider = gtk::CssProvider::new();
            provider.load_from_resource("/io/github/nico359/loci/style.css");
            gtk::style_context_add_provider_for_display(
                &gdk::Display::default().expect("no display"),
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // We connect to the activate callback to create a window when the application
        // has been launched. Additionally, this callback notifies us when the user
        // tries to launch a "second instance" of the application. When they try
        // to do that, we'll just present any existing window.
        fn activate(&self) {
            let application = self.obj();
            // Get the current window or create one if necessary
            let window = application.active_window().unwrap_or_else(|| {
                let window = LociWindow::new(&*application);
                window.upcast()
            });

            // Ask the window manager/compositor to present the window
            window.present();
        }
    }

    impl GtkApplicationImpl for LociApplication {}
    impl AdwApplicationImpl for LociApplication {}
}

glib::wrapper! {
    pub struct LociApplication(ObjectSubclass<imp::LociApplication>)
        @extends gio::Application, gtk::Application, adw::Application,
        @implements gio::ActionGroup, gio::ActionMap;
}

impl LociApplication {
    pub fn new(application_id: &str, flags: &gio::ApplicationFlags) -> Self {
        glib::Object::builder()
            .property("application-id", application_id)
            .property("flags", flags)
            .property("resource-base-path", "/io/github/nico359/loci")
            .build()
    }

    fn setup_gactions(&self) {
        let quit_action = gio::ActionEntry::builder("quit")
            .activate(move |app: &Self, _, _| app.quit())
            .build();
        let about_action = gio::ActionEntry::builder("about")
            .activate(move |app: &Self, _, _| app.show_about())
            .build();
        self.add_action_entries([quit_action, about_action]);
    }

    fn show_about(&self) {
        let window = self.active_window().unwrap();
        let about = adw::AboutDialog::builder()
            .application_name("Loci")
            .application_icon("io.github.nico359.loci")
            .developer_name("nico359")
            .version(VERSION)
            .developers(vec!["nico359", "GitHub Copilot CLI (Claude)"])
            .comments("A turn-by-turn navigation app for mobile Linux.\n\nBuilt with the assistance of AI (GitHub Copilot CLI, powered by Claude).")
            .website("https://github.com/nico359/loci")
            .issue_url("https://github.com/nico359/loci/issues")
            .license_type(gtk::License::Gpl30)
            // Translators: Replace "translator-credits" with your name/username, and optionally an email or URL.
            .translator_credits(&gettext("translator-credits"))
            .copyright("© 2026 nico359")
            .build();

        about.add_credit_section(
            Some(&gettext("Inspired by")),
            &["GNOME Maps by the GNOME Project https://apps.gnome.org/Maps/"],
        );

        about.present(Some(&window));
    }
}
