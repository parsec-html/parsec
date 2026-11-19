// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Typed values: CSS Values and Units Level 4's numbers, dimensions and
//! math functions, CSS Color Level 4's colors, and the value definition
//! syntax every property's grammar is written in.

mod canonical;
mod color;
mod convert;
mod custom;
mod generated;
mod grammar;
mod matcher;
mod math;
mod path;
mod properties;
mod restrict;
mod units;
mod value;

pub use color::{Channel, Channels, Color, Model, Predefined, Relative, RelativeChannel};
pub use convert::{Absolute, Space};
pub use custom::{CustomProperties, GuaranteedInvalid, Resolved, substitute, substitute_value};
pub use grammar::Grammar;
pub use math::{Expect, Leaf, Math, MathFunction, Node, Rounding};
pub use properties::{Property, PropertyData, properties, property};
pub use units::{Kind, Unit, number_to_string, write_number};
pub use value::{
    Component, CssWide, InvalidValue, Specified, expand, parse_declaration_value, parse_value,
    write_components,
};
