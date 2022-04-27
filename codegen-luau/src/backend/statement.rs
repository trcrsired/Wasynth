use std::{
	io::{Result, Write},
	ops::Range,
};

use wasm_ast::node::{
	Backward, Br, BrIf, BrTable, Call, CallIndirect, Else, Forward, If, Intermediate, Memorize,
	Return, SetGlobal, SetLocal, Statement, StoreAt,
};

use crate::analyzer::memory;

use super::manager::{write_ascending, write_separated, write_variable, Driver, Label, Manager};

fn br_target(level: usize, in_loop: bool, w: &mut dyn Write) -> Result<()> {
	write!(w, "if desired then ")?;
	write!(w, "if desired == {level} then ")?;
	write!(w, "desired = nil ")?;

	if in_loop {
		write!(w, "continue ")?;
	}

	write!(w, "end ")?;
	write!(w, "break ")?;
	write!(w, "end ")
}

fn write_br_gadget(label_list: &[Label], rem: usize, w: &mut dyn Write) -> Result<()> {
	match label_list.last() {
		Some(Label::Forward | Label::If) => br_target(rem, false, w),
		Some(Label::Backward) => br_target(rem, true, w),
		None => Ok(()),
	}
}

impl Driver for Memorize {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "reg_{} = ", self.var)?;
		self.value.write(mng, w)
	}
}

impl Driver for Forward {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		let rem = mng.push_label(Label::Forward);

		write!(w, "while true do ")?;

		self.body.iter().try_for_each(|s| s.write(mng, w))?;

		write!(w, "break ")?;
		write!(w, "end ")?;

		mng.pop_label();
		write_br_gadget(mng.label_list(), rem, w)
	}
}

impl Driver for Backward {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		let rem = mng.push_label(Label::Backward);

		write!(w, "while true do ")?;

		self.body.iter().try_for_each(|s| s.write(mng, w))?;

		write!(w, "break ")?;
		write!(w, "end ")?;

		mng.pop_label();
		write_br_gadget(mng.label_list(), rem, w)
	}
}

impl Driver for Else {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "else ")?;

		self.body.iter().try_for_each(|s| s.write(mng, w))
	}
}

impl Driver for If {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		let rem = mng.push_label(Label::If);

		write!(w, "while true do ")?;
		write!(w, "if ")?;
		self.cond.write(mng, w)?;
		write!(w, "~= 0 then ")?;

		self.truthy.iter().try_for_each(|s| s.write(mng, w))?;

		if let Some(s) = &self.falsey {
			s.write(mng, w)?;
		}

		write!(w, "end ")?;
		write!(w, "break ")?;
		write!(w, "end ")?;

		mng.pop_label();
		write_br_gadget(mng.label_list(), rem, w)
	}
}

fn write_br_at(up: usize, mng: &Manager, w: &mut dyn Write) -> Result<()> {
	write!(w, "do ")?;

	if up == 0 {
		if let Some(&Label::Backward) = mng.label_list().last() {
			write!(w, "continue ")?;
		} else {
			write!(w, "break ")?;
		}
	} else {
		let level = mng.label_list().len() - 1 - up;

		write!(w, "desired = {level} ")?;
		write!(w, "break ")?;
	}

	write!(w, "end ")
}

impl Driver for Br {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_br_at(self.target, mng, w)
	}
}

impl Driver for BrIf {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "if ")?;
		self.cond.write(mng, w)?;
		write!(w, "~= 0 then ")?;
		write_br_at(self.target, mng, w)?;
		write!(w, "end ")
	}
}

impl Driver for BrTable {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "do ")?;
		write!(w, "local temp = {{")?;

		if !self.data.table.is_empty() {
			write!(w, "[0] =")?;

			for d in self.data.table.iter() {
				write!(w, "{d}, ")?;
			}
		}

		write!(w, "}} ")?;

		write!(w, "desired = temp[")?;
		self.cond.write(mng, w)?;
		write!(w, "] or {} ", self.data.default)?;
		write!(w, "break ")?;
		write!(w, "end ")
	}
}

impl Driver for Return {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "do return ")?;
		self.list.as_slice().write(mng, w)?;
		write!(w, "end ")
	}
}

fn write_call_store(result: Range<usize>, w: &mut dyn Write) -> Result<()> {
	if result.is_empty() {
		return Ok(());
	}

	write_ascending("reg", result, w)?;
	write!(w, " = ")
}

impl Driver for Call {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_call_store(self.result.clone(), w)?;

		write!(w, "FUNC_LIST[{}](", self.func)?;
		self.param_list.as_slice().write(mng, w)?;
		write!(w, ")")
	}
}

impl Driver for CallIndirect {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_call_store(self.result.clone(), w)?;

		write!(w, "TABLE_LIST[{}].data[", self.table)?;
		self.index.write(mng, w)?;
		write!(w, "](")?;
		self.param_list.as_slice().write(mng, w)?;
		write!(w, ")")
	}
}

impl Driver for SetLocal {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_variable(self.var, mng, w)?;
		write!(w, "= ")?;
		self.value.write(mng, w)
	}
}

impl Driver for SetGlobal {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "GLOBAL_LIST[{}].value = ", self.var)?;
		self.value.write(mng, w)
	}
}

impl Driver for StoreAt {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "store_{}(memory_at_0, ", self.what.as_name())?;
		self.pointer.write(mng, w)?;
		write!(w, "+ {}, ", self.offset)?;
		self.value.write(mng, w)?;
		write!(w, ")")
	}
}

impl Driver for Statement {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		match self {
			Self::Unreachable => write!(w, "error(\"out of code bounds\")"),
			Self::Memorize(s) => s.write(mng, w),
			Self::Forward(s) => s.write(mng, w),
			Self::Backward(s) => s.write(mng, w),
			Self::If(s) => s.write(mng, w),
			Self::Br(s) => s.write(mng, w),
			Self::BrIf(s) => s.write(mng, w),
			Self::BrTable(s) => s.write(mng, w),
			Self::Return(s) => s.write(mng, w),
			Self::Call(s) => s.write(mng, w),
			Self::CallIndirect(s) => s.write(mng, w),
			Self::SetLocal(s) => s.write(mng, w),
			Self::SetGlobal(s) => s.write(mng, w),
			Self::StoreAt(s) => s.write(mng, w),
		}
	}
}

fn write_parameter_list(ir: &Intermediate, w: &mut dyn Write) -> Result<()> {
	write!(w, "function(")?;
	write_ascending("param", 0..ir.num_param, w)?;
	write!(w, ")")
}

fn write_variable_list(ir: &Intermediate, w: &mut dyn Write) -> Result<()> {
	let mut total = 0;

	for data in &ir.local_data {
		let range = total..total + usize::try_from(data.count()).unwrap();
		let typed = data.value_type();

		total = range.end;

		write!(w, "local ")?;
		write_ascending("loc", range.clone(), w)?;
		write!(w, " = ")?;
		write_separated(range, |_, w| write!(w, "ZERO_{typed} "), w)?;
	}

	if ir.num_stack != 0 {
		write!(w, "local ")?;
		write_ascending("reg", 0..ir.num_stack, w)?;
		write!(w, " ")?;
	}

	Ok(())
}

impl Driver for Intermediate {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_parameter_list(self, w)?;

		for v in memory::visit(self) {
			write!(w, "local memory_at_{v} = MEMORY_LIST[{v}]")?;
		}

		write_variable_list(self, w)?;

		mng.num_param = self.num_param;
		self.code.write(mng, w)?;

		write!(w, "end ")
	}
}