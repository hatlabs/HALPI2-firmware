use postcard_bindgen::{PackageInfo, generate_bindings, python};

use shared_types::{
    FlashUpdateCommand, FlashUpdateResponse, FlashUpdateState,
};

fn main() {
    python::build_package(
        std::env::current_dir().unwrap().as_path(),
        PackageInfo {
            name: "halpi2-fw-i2c-postcard".into(),
            version: "0.1.0".try_into().unwrap(),
        },
        python::GenerationSettings::enable_all(),
        generate_bindings!(FlashUpdateCommand, FlashUpdateResponse, FlashUpdateState),
    )
    .unwrap();
}
