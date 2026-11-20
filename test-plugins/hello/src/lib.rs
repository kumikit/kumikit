//! The smallest plugin: logs on activation. Used by plugin-host tests.

use kumikit_plugin::{Error, Plugin, log};

struct Hello;

impl Plugin for Hello {
    fn activate() -> Result<(), Error> {
        log::log(log::Level::Info, "hello activated");
        Ok(())
    }

    fn deactivate() {}
}

kumikit_plugin::export_plugin!(Hello);
