#[path = "../windows/build_support/resources.rs"]
mod windows_resources;

fn main() {
    windows_resources::embed("winmail-rust");
}
