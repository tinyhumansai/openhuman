fn main() {
    println!("cargo:rerun-if-changed=permissions");
    println!("cargo:rerun-if-changed=capabilities");
    tauri_build::build();
}
