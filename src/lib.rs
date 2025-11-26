// This crate is a drop-in replacement for serde_json, but with better error
// messages (specifically, it mentions the name of the field that failed to
// parse).
//
// For this entire library, remap the serde_json crate to use it instead:
extern crate serde_json_path_to_error as serde_json;

mod handle_raw {
    pub mod macros;
}
mod handle_with_function {
    pub mod macros;
}
mod handle_with_router {
    pub mod macros;
    pub mod routing_config;
    pub mod std {
        pub mod crud_specs;
        pub mod function_specs;
        pub mod validators;
    }
}
mod shared {
    pub mod auth_utils;
    pub mod request_processing;
    pub mod response_building;
}

mod constants;
mod errors;

//
// ===========================================================================
// External interface:
// ---------------------------------------------------------------------------

// (Macros are automatically exported at crate root.)

pub use errors::*;
pub use handle_with_router::routing_config::*;
pub use handle_with_router::std::crud_specs::*;
pub use handle_with_router::std::function_specs::*;
pub use handle_with_router::std::validators::*;
pub use shared::request_processing::*;
pub use shared::response_building::*;

// ---------------------------------------------------------------------------
