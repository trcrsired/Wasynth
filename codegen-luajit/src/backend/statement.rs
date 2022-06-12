use std::{
	io::{Result, Write},
	ops::Range,
};

use parity_wasm::elements::ValueType;
use wasm_ast::node::{
	Backward, Br, BrTable, Call, CallIndirect, Forward, FuncData, If, SetGlobal, SetLocal,
	SetTemporary, Statement, StoreAt, Terminator,
};

use super::manager::{
	write_ascending, write_condition, write_separated, write_variable, Driver, Manager,
};

fn write_br_at(up: usize, mng: &Manager, w: &mut dyn Write) -> Result<()> {
	let level = mng.label_list().iter().nth_back(up).unwrap();

	write!(w, "goto continue_at_{level} ")
}

impl Driver for Br {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_br_at(self.target, mng, w)
	}
}

fn condense_jump_table(list: &[u32]) -> Vec<(usize, usize, u32)> {
	let mut result = Vec::new();
	let mut index = 0;

	while index < list.len() {
		let start = index;

		loop {
			index += 1;

			// if end of list or next value is not equal, break
			if index == list.len() || list[index - 1] != list[index] {
				break;
			}
		}

		result.push((start, index - 1, list[start]));
	}

	result
}

impl Driver for BrTable {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		let default = self.data.default.try_into().unwrap();

		// Our condition should be pure so we probably don't need
		// to emit it in this case.
		if self.data.table.is_empty() {
			return write_br_at(default, mng, w);
		}

		write!(w, "temp = ")?;
		self.cond.write(mng, w)?;

		for (start, end, dest) in condense_jump_table(&self.data.table) {
			if start == end {
				write!(w, "if temp == {start} then ")?;
			} else {
				write!(w, "if temp >= {start} and temp <= {end} then ")?;
			}

			write_br_at(dest.try_into().unwrap(), mng, w)?;
			write!(w, "else")?;
		}

		write!(w, " ")?;
		write_br_at(default, mng, w)?;
		write!(w, "end ")
	}
}

impl Driver for Terminator {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		match self {
			Self::Unreachable => write!(w, "error(\"out of code bounds\")"),
			Self::Br(s) => s.write(mng, w),
			Self::BrTable(s) => s.write(mng, w),
		}
	}
}

impl Driver for Forward {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		let label = mng.push_label();

		self.code.iter().try_for_each(|s| s.write(mng, w))?;

		if let Some(v) = &self.last {
			v.write(mng, w)?;
		}

		write!(w, "::continue_at_{label}::")?;

		mng.pop_label();

		Ok(())
	}
}

impl Driver for Backward {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		let label = mng.push_label();

		write!(w, "::continue_at_{label}::")?;
		write!(w, "while true do ")?;

		self.code.iter().try_for_each(|s| s.write(mng, w))?;

		if let Some(v) = &self.last {
			v.write(mng, w)?;
		}

		write!(w, "break ")?;
		write!(w, "end ")?;

		mng.pop_label();

		Ok(())
	}
}

impl Driver for If {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "if ")?;
		write_condition(&self.cond, mng, w)?;
		write!(w, "then ")?;

		self.truthy.write(mng, w)?;

		if let Some(falsey) = &self.falsey {
			write!(w, "else ")?;

			falsey.write(mng, w)?;
		}

		write!(w, "end ")?;

		Ok(())
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

impl Driver for SetTemporary {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write!(w, "reg_{} = ", self.var)?;
		self.value.write(mng, w)
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

		if self.offset != 0 {
			write!(w, "+ {}", self.offset)?;
		}

		write!(w, ", ")?;
		self.value.write(mng, w)?;
		write!(w, ")")
	}
}

impl Driver for Statement {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		match self {
			Self::Forward(s) => s.write(mng, w),
			Self::Backward(s) => s.write(mng, w),
			Self::If(s) => s.write(mng, w),
			Self::Call(s) => s.write(mng, w),
			Self::CallIndirect(s) => s.write(mng, w),
			Self::SetTemporary(s) => s.write(mng, w),
			Self::SetLocal(s) => s.write(mng, w),
			Self::SetGlobal(s) => s.write(mng, w),
			Self::StoreAt(s) => s.write(mng, w),
		}
	}
}

fn write_parameter_list(ast: &FuncData, w: &mut dyn Write) -> Result<()> {
	write!(w, "function(")?;
	write_ascending("param", 0..ast.num_param, w)?;
	write!(w, ")")
}

fn write_variable_list(ast: &FuncData, w: &mut dyn Write) -> Result<()> {
	let mut total = 0;

	for data in &ast.local_data {
		let range = total..total + usize::try_from(data.count()).unwrap();
		let typed = if data.value_type() == ValueType::I64 {
			"0LL"
		} else {
			"0"
		}
		.as_bytes();

		total = range.end;

		write!(w, "local ")?;
		write_ascending("loc", range.clone(), w)?;
		write!(w, " = ")?;
		write_separated(range, |_, w| w.write_all(typed), w)?;
		write!(w, " ")?;
	}

	if ast.num_stack != 0 {
		write!(w, "local ")?;
		write_ascending("reg", 0..ast.num_stack, w)?;
		write!(w, " ")?;
	}

	Ok(())
}

impl Driver for FuncData {
	fn write(&self, mng: &mut Manager, w: &mut dyn Write) -> Result<()> {
		write_parameter_list(self, w)?;
		write_variable_list(self, w)?;
		write!(w, "local temp ")?;

		mng.num_param = self.num_param;
		self.code.write(mng, w)?;

		if self.num_result != 0 {
			write!(w, "return ")?;
			write_ascending("reg", 0..self.num_result, w)?;
			write!(w, " ")?;
		}

		write!(w, "end ")
	}
}
