use std::env::args;

pub mod switcher;
pub mod compat;

use crate::switcher::{ Config, System, sw_device::Error };
use std::fs;

#[cfg_attr(feature = "async-std", async_std::main)]
async fn main() -> Result<(), Error> {

	let config = match args().nth(1) {
		None => "./config.json".to_string(),
		Some(s) => s
	};

	let data = fs::read_to_string(config).expect("Unable to read file");

	match serde_json::from_str::<Config>(&data) {
		Ok(c) => {
			let mut s = System::new(c);

			//main loop
			s.run().await?;
		},
		Err(e) => {
			eprintln!("Bad config file: {}", e);
		}
	}

	Ok(())
}
