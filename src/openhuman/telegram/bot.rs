fn handle_operator_approval(message: &TelegramMessage) -> Result<()> {
    // Verificar si ya hay una solicitud pendiente para este usuario
    if let Some(pending) = get_pending_approval(message.from.id) {
        // Si ya hay una solicitud pendiente, no crear una nueva
        return Ok(());
    }
    
    // Crear una nueva solicitud de aprobación
    create_approval_request(message.from.id, message.text)?;
    
    Ok(())
}