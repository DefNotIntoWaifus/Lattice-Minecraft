//! Checks whether Microsoft accepts a client ID for the device-code flow:
//!   cargo run -p lattice-auth --example check_client -- <client-id>
//! It only requests a code (nothing is signed in) and prints the result.
fn main() {
    let id = std::env::args().nth(1).unwrap_or_default();
    match lattice_auth::start_device_code(&id) {
        Ok(c) => println!("accepted: go to {} and enter {} (not needed for this check)", c.verification_uri, c.user_code),
        Err(e) => println!("error: {e}"),
    }
}
