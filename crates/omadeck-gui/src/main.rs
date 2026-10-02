//! omadeck — Stream Deck configurator for Omarchy.

mod app;
mod apps;
mod editor;
mod media;
mod starter;

use adw::prelude::*;

pub const APP_ID: &str = "dev.omadeck.Omadeck";

fn main() -> gtk::glib::ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).format_timestamp(None).init();
    let application = adw::Application::builder().application_id(APP_ID).build();
    application.connect_activate(|application| {
        // Single window: re-activating just raises it.
        if let Some(win) = application.active_window() {
            win.present();
            return;
        }
        app::App::build(application);
    });
    application.run()
}
