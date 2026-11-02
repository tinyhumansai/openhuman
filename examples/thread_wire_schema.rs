fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!(
        "{}",
        openhuman_core::threads::wire_schema::thread_wire_schema_json()?
    );
    Ok(())
}
