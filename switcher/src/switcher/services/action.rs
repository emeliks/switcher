use serde::{ Deserialize };
use crate::compat::{ sync::{ Mutex }, task };
use std::{ collections::{ HashMap }, sync::Arc };
use core::{ ops::{ DerefMut } };
use tokenizer::Eval;
use crate::{
	switcher::{ SwEvaluator, SwMessage, System, sw_device::{ Error }, SwEvalCtx, SwValue, SwDeviceInstance },
};

#[derive(Deserialize, Clone, Debug)]
pub struct SwAction {
	on: Option<SwEvaluator>,
	#[serde(rename = "do")]
	tdo: SwEvaluator,
	#[serde(skip_deserializing)]
	vals: Arc<Mutex<HashMap<String, SwValue>>>,
	#[serde(skip_deserializing)]
	dev: Option<(Arc<SwDeviceInstance>, usize)>
}

impl SwAction {
	pub fn set_dev(&mut self, a: &Arc<SwDeviceInstance>, sender_id: usize) {
		self.dev = Some((a.clone(), sender_id));
	}

	pub async fn run(self: Arc<Self>, system: &mut System) -> Result<(), Error> {
		let (sender_id, rx, tx) = system.get_channel();

		let mut ctx = SwEvalCtx{ ..Default::default() };
		let mut obs = HashMap::new();

		//collect observed attrs
		let mut v = |e: &SwEvaluator, eval_on: bool| if let SwEvaluator::Val{ n, a1, .. } = e {

			if *n == 1 {
				if let Ok(SwValue::String(s)) = a1.eval(&mut ctx) {
					if let (None, Some((dev, _))) = (&s.find('.'), &self.dev) {
						//if action in device context - complete name with dev id prefix

						let name = format!("{}.{}", dev.id, s);

						if !obs.contains_key(&name) {
							//in case of attr exists both in on and do, do not overrwrite previous value
							obs.insert(name, eval_on);
						}
					}
					else {
						if !obs.contains_key(&s) {
							obs.insert(s, eval_on);
						}
					}
				}
			}
		};

		let mut eval_on = true;
		let mut vx = |e: &SwEvaluator| v(e, eval_on);

		if let Some(on) = &self.on {
			//on condition do eval on
			on.visit(&mut vx);
			eval_on = false;
		}

		let mut vx = |e: &SwEvaluator| v(e, eval_on);

		//only remember value
		self.tdo.visit(&mut vx);

		for (s, _) in obs.iter() {
			tx.send(SwMessage::ObserveAttr{sender_id, name: s.clone()}).await?;
		}

		//retain only values to eval
		obs.retain(|_, &mut v| v);

		let vals = self.vals.clone();
		let a = self.clone();
		let dev = self.dev.clone();

		#[allow(unreachable_code)]
		task::spawn(async move {
			loop {
				if let Err(e) = async {
					let received = rx.recv().await?;

					match received {
						SwMessage::DevAttr{ sender_id: _, name, value } => {
							let mut ctx = SwEvalCtx{ msgs: Some(Vec::new()), ..Default::default() };

							if let Some((ref dev, sender_id)) = dev {
								ctx.vals_prefix = Some(dev.id.clone());
								ctx.dev = Some((dev.clone(), sender_id));
							}

							let eval_on = obs.get(&name);
							let mut cmsgs = None;

							{
								let mut v = vals.lock().await;
								v.insert(name, value);

								//steal vals from mutex
								let sv = std::mem::take(v.deref_mut());
								ctx.vals = Some(sv);

								if let Some(true) = eval_on {
									//eval on
									let is_true = match &a.on {
										None => true,
										Some(w) => match w.eval(&mut ctx) {
											Ok(r) => match r.into_bool() {
												Ok(b) => b,
												Err(e) => {
													println!("Error in action: {}", e);
													false
												}
											},
											Err(e) => {
												println!("Error in action: {}", e);
												false
											}
										}
									};

									if is_true {
										_ = &a.tdo.eval(&mut ctx);
									}
								}

								//should current v be empty?

								if let SwEvalCtx{ vals: Some(sv), mut msgs, .. } = ctx {
									//give back
									_ = std::mem::replace(v.deref_mut(), sv);
									cmsgs = std::mem::take(&mut msgs);
								}
							}

							if let Some(msgs) = cmsgs {
								//send msgs
								for msg in msgs {
									tx.send(msg).await?;
								}
							}
						},
						_ => {}
					}
					Ok::<(), Error>(())
				}.await {
					println!("Error in action loop: {}", e);
				}
			}
		});

		Ok(())
	}
}