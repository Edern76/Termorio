use super::errors::ProgError;
use std::env;

pub fn get_program_name() -> Result<String, ProgError> {
    Ok(env::current_exe()?
        .file_name()
        .ok_or(ProgError::NoFile)?
        .to_str()
        .ok_or(ProgError::NotUtf8)?
        .to_owned())
}
