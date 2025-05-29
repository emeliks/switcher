use core::{ cmp::{ PartialOrd }, fmt::{ Display, Formatter, Debug }, num::{ ParseFloatError, ParseIntError }, convert::{ Infallible, TryInto, From }, ops::{ Add, Neg, Sub, Mul, Div, BitAnd, BitOr } };

//expression function

#[derive(Debug)]
pub enum Error {
	BadArgument,
	TooFewTokens,
	UnexpectedToken(&'static str),
	EmptyStack,
	TooFewArguments((String, usize, usize)),
	BadType(&'static str),
	String(String),
	ParseFloatError(ParseFloatError),
	ParseIntError(ParseIntError),
	Infallible(Infallible)
}

impl From<ParseFloatError> for Error {
	fn from(r: ParseFloatError) -> Self {
		Error::ParseFloatError(r)
	}
}

impl From<ParseIntError> for Error {
	fn from(r: ParseIntError) -> Self {
		Error::ParseIntError(r)
	}
}

impl From<Infallible> for Error {
	fn from(i: Infallible) -> Self {
		Error::Infallible(i)
	}
}

impl Display for Error {
	fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
		match self {
			Self::BadArgument => write!(f, "Bad argument expression"),
			Self::TooFewTokens => write!(f, "Too few tokens"),
			Self::UnexpectedToken(s) => write!(f, "Unexpected token: {}", s),
			Self::EmptyStack => write!(f, "Empty stack"),
			Self::TooFewArguments((s, n, m)) => write!(f, "Too few arguments in {}, required at least {}, actual {}", s, n, m),
			Self::BadType(s) => write!(f, "Bad type: {}", s),
			Self::String(s) => write!(f, "Bad type: {}", s),
			Self::ParseFloatError(p) => write!(f, "Parse float error: {}", p),
			Self::ParseIntError(p) => write!(f, "Parse int error: {}", p),
			Self::Infallible(i) => write!(f, "Infallible: {}", i),
		}
	}
}

//tokenize traits

//evaluator: function or operator

#[derive(Debug, Clone)]
pub enum Assoc {
	Left,
	Right
}

#[derive(Debug, Clone)]
pub enum TokenType {
	Literal,
	Operator{ precedence: u16, assoc: Assoc },
	Function
}

pub trait Eval: Default {
	type EvalRes: Clone + PartialEq + Debug + From<String> + Into<Self>;
	type EvalCtx;
	type Error;

	fn match_str(s: &str, ahead: bool) -> Option<(Self, usize)> where Self: Sized;

	fn get_token_type(&self) -> TokenType;
	fn set_arg_num(&mut self, num: usize);
	fn build(&mut self, tokens: &mut Vec<Token<Self>>) -> Result<(), Self::Error> where Self: Sized;
	fn eval(&self, ctx: &mut Self::EvalCtx) -> Result<Self::EvalRes, Self::Error> where <Self as Eval>::EvalRes: Clone;

	fn def() -> Box<Self> {
		Box::new(Self::default())
	}
}

#[derive(Debug)]
pub enum Token<'a, E: Eval> {
	LeftParenthesis,
	RightParenthesis,
	Comma,
	Identifier(&'a str),
	Eval(E)
}

#[derive(Debug)]
pub struct Tokenizer<'a, E: Eval + Debug> {
	pub string_marker: char,
	expr: &'a str,
	lnds: usize, //last non delim start
	lnde: usize, //last non delim end
	next_token: Option<Token<'a, E>>,
	pos: usize
}

impl<'a, E: Eval + Debug> Iterator for Tokenizer<'a, E> {
	type Item = Token<'a, E>;

	fn next(&mut self) -> Option<Self::Item> {
		if let Some(token) = std::mem::take(&mut self.next_token) {
			return Some(token);
		}

		loop {
			let mut chars = self.expr[self.pos..].chars().peekable();
			let sc = chars.next();
			let mut processed_len = 0;
			let mut is_delim = false;
			let mut cur_token = None;

			if let Some(c) = sc {
				processed_len = c.len_utf8();

				//whitespaces
				if c.is_whitespace() {
					is_delim = true;

					//eat next whitespaces
					while let Some(c) = chars.peek() {
						if c.is_whitespace(){
							processed_len += c.len_utf8();
							chars.next();
						}
						else {
							break;
						}
					}
				}
				else {
					if c == self.string_marker {
						let f = self.pos + c.len_utf8();
						let mut to = f;

						while let Some(c) = chars.peek() {
							let l = c.len_utf8();
							processed_len += l;

							if *c != self.string_marker {
								to += l;
								chars.next();
							}
							else {
								break;
							}
						}

						is_delim = true;
						cur_token = Some(Token::Eval(E::EvalRes::from(self.expr[f..to].to_string()).into()));
					}
					else {
						//match operator

						match c {
							'(' => { is_delim = true; cur_token = Some(Token::LeftParenthesis); },
							')' => { is_delim = true; cur_token = Some(Token::RightParenthesis); },
							',' => { is_delim = true; cur_token = Some(Token::Comma); },
							_ => {
								//only operator ahead
								if let Some((e, l)) = E::match_str(&self.expr[self.pos..], true) {
									if let TokenType::Operator{ .. } = e.get_token_type() {
										processed_len = l;
										cur_token = Some(Token::Eval(e));
										is_delim = true;
									}
								}
							}
						}
					}
				}
			}

			self.pos += processed_len;

			let lnds = self.lnds;
			let lnde = self.lnde;

			#[allow(unused_assignments)]
			if is_delim {
				self.lnds = self.pos;
				self.lnde = self.lnds;

				if let Some(token) = cur_token {
					cur_token = None;

					if lnds == lnde {
						return Some(token);
					}
					else {
						self.next_token = Some(token);
					}
				}
			}
			else {
				self.lnde += processed_len;
			}

			if (is_delim || processed_len == 0) && lnds != lnde {
				self.lnds = self.pos;
				self.lnde = self.lnds;

				//function or operator or literal

				if let Some((e, l)) = E::match_str(&self.expr[lnds..lnde], false) {
					if l == lnde - lnds {
						return Some(Token::Eval(e));
					}
				}

				//identifier

				return Some(Token::Identifier(&self.expr[lnds..lnde]));
			}

			if processed_len == 0 {
				break;
			}
		}

		None
	}
}

impl<'a, E: Eval + Debug> Tokenizer<'a, E> {
	pub fn new(expr: &'a str) -> Self {
		Tokenizer {
			string_marker: '\"',
			expr: expr,
			lnds: 0,
			lnde: 0,
			next_token: None,
			pos: 0
		}
	}

	pub fn get_expr(tokens: &mut Vec<Token<E>>) -> Result<E, Error>
		where <E as Eval>::Error: Display
	{
		if let Some(token) = tokens.pop() {
			match token {
				Token::Eval(mut e) => {
					if let Err(e) = e.build(tokens) {
						return Err(Error::String(format!("Error while build expression: {}", e)));
					}

					return Ok(e);
				},
				_ => {
					return Err(Error::UnexpectedToken("Expected eval token in get_expr"));
				}
			}
		}

		Err(Error::EmptyStack)
	}

	//converts tokens to Reverse Polish Notation

	pub fn rpn(&mut self) -> Result<Vec<Token<E>>, Error> {
		let mut out: Vec<Token<E>> = Vec::new();
		let mut stack: Vec<Token<E>> = Vec::new();
		//number of commas, was anything between parentheses
		let mut a_stack: Vec<(usize, usize)> = Vec::new();

		while let Some(token) = self.next() {
			if let Token::RightParenthesis = token {
			} else {
				if let Some((_n, pars)) = a_stack.last_mut() {
					if *pars == 0 {
						*pars = 1;
					}
				}
			}

			match token {
				Token::Comma => {
					if let Some((n, _)) = a_stack.last_mut() {
						*n += 1;
					}

					while let Some(t) = stack.last() {
						if let Token::LeftParenthesis = t {
							break;
						}

						out.push(stack.pop().unwrap());
					}
				},
				Token::Eval(ref e) => {
					match e.get_token_type() {
						TokenType::Literal => {
							out.push(token);
						},
						TokenType::Function => {
							stack.push(token);
						},
						TokenType::Operator{ precedence: op_prior, ref assoc } => {
							loop {
								if let Some(Token::Eval(oe)) = stack.last() {
									if let TokenType::Operator{ precedence: oop_prior, assoc: _ } = oe.get_token_type() {
										let op_left = if let Assoc::Left = assoc { true } else { false };

										if op_left && op_prior <= oop_prior || !op_left && op_prior < oop_prior {
											out.push(stack.pop().unwrap());
											continue;
										}
									}
								}

								break;
							}

							stack.push(token);
						}
					}
				},
				Token::LeftParenthesis => {
					stack.push(token);
					a_stack.push((0, 0));
				},
				Token::RightParenthesis => {
					let a_num = if let Some((n, pars)) = a_stack.pop() { n + pars } else { 0 };

					while let Some(top) = stack.pop() {
						if let Token::LeftParenthesis = top {
							break;
						}
						else {
							out.push(top);
						}
					}

					if let Some(sl) = stack.last_mut() {
						if let Token::Eval(ref mut e) = sl {
							if let TokenType::Function = e.get_token_type() {
								e.set_arg_num(a_num);
								out.push(stack.pop().unwrap());
							}
							else {
								//operator with variable number of rhs (ie. in (a, b, c, ...))
								e.set_arg_num(a_num);
							}
						}
					}
				},
				_ => out.push(token)
			}
		}

		while let Some(token) = stack.pop() {
			out.push(token);
		}

		Ok(out)
	}

	pub fn ast(&mut self) -> Result<E, Error>
		where <E as Eval>::Error: Display
	{
		Self::get_expr(&mut self.rpn()?)
	}
}

//std eval - basic operators

#[derive(Clone, Debug)]
pub enum StdEval<E: Eval + Default + Debug> {
	BoolAnd{ a1: E, a2: E },
	BoolOr{ a1: E, a2: E },
	BoolNeg{ a: E },
	BitAnd{ a1: E, a2: E },
	BitOr{ a1: E, a2: E },
	Eq{ a1: E, a2: E },
	Neq{ a1: E, a2: E },
	Neg{ a: E },
	Add{ a1: E, a2: E },
	Sub{ a1: E, a2: E },
	Mul{ a1: E, a2: E },
	Div{ a1: E, a2: E },
	Gt{ a1: E, a2: E },
	Lt{ a1: E, a2: E },
	None
}

impl<E: Eval + Default + Debug> Default for StdEval<E> {
	fn default() -> StdEval<E> {
		StdEval::None
	}
}

//not impl eval because of conflict in build function, in trait tokens is: &mut Vec<Token<Self>>, we have tokens: &mut Vec<Token<E>>

impl<E: Eval + Default + Debug> StdEval<E> where
	E::EvalRes: Add + Neg + Sub + Mul + Div + BitAnd + BitOr + PartialOrd + From<bool> + TryInto<bool>
{
	pub fn build(&mut self, tokens: &mut Vec<Token<E>>) -> Result<(), E::Error> where
		E::Error: From<Error> + Display
	{
		match self {
			StdEval::BoolNeg{ a } |
			StdEval::Neg{ a } => {
				*a = Tokenizer::<E>::get_expr(tokens)?;
			},
			StdEval::BoolAnd{ a1, a2 } |
			StdEval::BoolOr{ a1, a2 } |
			StdEval::BitAnd{ a1, a2 } |
			StdEval::BitOr{ a1, a2 } |
			StdEval::Eq{ a1, a2 } |
			StdEval::Neq{ a1, a2 } |
			StdEval::Add{ a1, a2 } |
			StdEval::Sub{ a1, a2 } |
			StdEval::Mul{ a1, a2 } |
			StdEval::Lt{ a1, a2 } |
			StdEval::Gt{ a1, a2 } |
			StdEval::Div{ a1, a2 } => {
				*a2 = Tokenizer::<E>::get_expr(tokens)?;
				*a1 = Tokenizer::<E>::get_expr(tokens)?;
			},
			_ => {}
		}

		Ok(())
	}

	pub fn match_str(s: &str, ahead: bool) -> Option<(Self, usize)> {
		let l = s.len();

		if l >= 1 {
			match &s[..1] {
				"+" => { return Some((StdEval::Add{ a1: E::default(), a2: E::default() }, 1)); },
				"-" => { return Some((StdEval::Sub{ a1: E::default(), a2: E::default() }, 1)); },	//todo neg
				"*" => { return Some((StdEval::Mul{ a1: E::default(), a2: E::default() }, 1)); },
				"/" => { return Some((StdEval::Div{ a1: E::default(), a2: E::default() }, 1)); },
				"~" => { return Some((StdEval::Neg{ a: E::default() }, 1)); },
				_ => {
					if l >= 2 {
						match &s[..2] {
							"&&" => { return Some((StdEval::BoolAnd{ a1: E::default(), a2: E::default() }, 2)); },
							"||" => { return Some((StdEval::BoolOr{ a1: E::default(), a2: E::default() }, 2)); },
							"==" => { return Some((StdEval::Eq{ a1: E::default(), a2: E::default() }, 2)); },
							"!=" => { return Some((StdEval::Neq{ a1: E::default(), a2: E::default() }, 2)); },
							_ => {
								match &s[..1] {
									//after matching two letter sized operator containing those
									"&" => { return Some((StdEval::BitAnd{ a1: E::default(), a2: E::default() }, 1)); },
									"|" => { return Some((StdEval::BitOr{ a1: E::default(), a2: E::default() }, 1)); },
									"!" => { return Some((StdEval::BoolNeg{ a: E::default() }, 1)); },
									">" => { return Some((StdEval::Gt{ a1: E::default(), a2: E::default() }, 1)); },
									"<" => { return Some((StdEval::Lt{ a1: E::default(), a2: E::default() }, 1)); },
									_ => {
										if l >= 3 {
											match &s[..3] {
												"not" => { return Some((StdEval::BoolNeg{ a: E::default() }, 3)); },
												_ => {}
											}
										}
									}
								}
							}
						}
					}
				}
			}
		}

		if !ahead {
			//only non alphanumeric operators in ahead mode

			//numeric literals in E::EvalRes
		}

		None
	}

	pub fn get_token_type(&self) -> TokenType {
		match self {
			Self::BoolAnd{ .. } => TokenType::Operator{ precedence: 7, assoc: Assoc::Left },
			Self::BoolOr{ .. } => TokenType::Operator{ precedence: 5, assoc: Assoc::Left },
			Self::BitAnd{ .. } => TokenType::Operator{ precedence: 4, assoc: Assoc::Left },
			Self::BitOr{ .. } => TokenType::Operator{ precedence: 3, assoc: Assoc::Left },
			Self::Lt{ .. } => TokenType::Operator{ precedence: 9, assoc: Assoc::Left },
			Self::Gt{ .. } => TokenType::Operator{ precedence: 9, assoc: Assoc::Left },
			Self::Eq{ .. } => TokenType::Operator{ precedence: 8, assoc: Assoc::Left },
			Self::Neq{ .. } => TokenType::Operator{ precedence: 8, assoc: Assoc::Left },
			Self::Neg{ .. } => TokenType::Operator{ precedence: 14, assoc: Assoc::Left },
			Self::BoolNeg{ .. } => TokenType::Operator{ precedence: 14, assoc: Assoc::Left },
			Self::Add{ .. } => TokenType::Operator{ precedence: 11, assoc: Assoc::Left },
			Self::Sub{ .. } => TokenType::Operator{ precedence: 11, assoc: Assoc::Left },
			Self::Mul{ .. } => TokenType::Operator{ precedence: 12, assoc: Assoc::Left },
			Self::Div{ .. } => TokenType::Operator{ precedence: 12, assoc: Assoc::Left },
			_ => TokenType::Literal
		}
	}

	fn as_bool(er: E::EvalRes) -> Result<bool, Error>
		where <<E as Eval>::EvalRes as TryInto<bool>>::Error: Display
	{
		match er.try_into() {
			Ok(b) => Ok(b),
			Err(e) => Err(Error::String(format!("Required bool val, found other: {}", e)))
		}
	}

	pub fn eval(&self, ctx: &mut E::EvalCtx) -> Result<E::EvalRes, E::Error> where
		E::EvalRes: BitAnd<Output = E::EvalRes> + BitOr<Output = E::EvalRes> + Neg<Output = E::EvalRes> + Add<Output = E::EvalRes> + Sub<Output = E::EvalRes> + Mul<Output = E::EvalRes> + Div<Output = E::EvalRes>,
		<<E as Eval>::EvalRes as TryInto<bool>>::Error: Display,
		<E as Eval>::Error: From<Error>
	{
		match self {
			Self::BoolAnd{ a1, a2 } => Ok((Self::as_bool(a1.eval(ctx)?)? && Self::as_bool(a2.eval(ctx)?)?).into()),
			Self::BoolOr{ a1, a2 } => Ok((Self::as_bool(a1.eval(ctx)?)? || Self::as_bool(a2.eval(ctx)?)?).into()),
			Self::BoolNeg{ a } => Ok((!Self::as_bool(a.eval(ctx)?)?).into()),
			Self::BitAnd{ a1, a2 } => Ok(a1.eval(ctx)? & a2.eval(ctx)?),
			Self::BitOr{ a1, a2 } => Ok(a1.eval(ctx)? | a2.eval(ctx)?),
			Self::Eq{ a1, a2 } => Ok((a1.eval(ctx)? == a2.eval(ctx)?).into()),
			Self::Neq{ a1, a2 } => Ok((a1.eval(ctx)? != a2.eval(ctx)?).into()),
			Self::Neg{ a } => Ok(-a.eval(ctx)?),
			Self::Add{ a1, a2 } => Ok(a1.eval(ctx)? + a2.eval(ctx)?),
			Self::Sub{ a1, a2 } => Ok(a1.eval(ctx)? - a2.eval(ctx)?),
			Self::Mul{ a1, a2 } => Ok(a1.eval(ctx)? * a2.eval(ctx)?),
			Self::Div{ a1, a2 } => Ok(a1.eval(ctx)? / a2.eval(ctx)?),
			Self::Lt{ a1, a2 } => Ok((a1.eval(ctx)? < a2.eval(ctx)?).into()),
			Self::Gt{ a1, a2 } => Ok((a1.eval(ctx)? > a2.eval(ctx)?).into()),
			Self::None => Err(E::Error::from(Error::BadType("None not evaluable")))
		}
	}
}
