use std::process::Command;

#[cfg(windows)]
pub(crate) fn hide_command_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x08000000);
}

#[cfg(not(windows))]
pub(crate) fn hide_command_window(_command: &mut Command) {}
