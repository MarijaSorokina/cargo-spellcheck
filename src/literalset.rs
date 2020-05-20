use crate::{Span,LineColumn};
use super::*;

use std::fs;

use indexmap::IndexMap;
use log::{debug, info, trace, warn};
use proc_macro2::{Spacing, TokenTree};

use std::path::{Path, PathBuf};





#[derive(Clone,Debug,Copy)]
/// A litteral with meta info where the first and list whitespace may be found.
pub struct AnnotatedLiteralRef<'l> {
    reference: &'l AnnotatedLiteral,
}

impl<'l> std::ops::Deref for AnnotatedLiteralRef<'l> {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.reference.literal
    }
}

impl<'l> From<&'l AnnotatedLiteral> for AnnotatedLiteralRef<'l> {
	fn from(anno: &'l AnnotatedLiteral) -> Self {
		Self {
			reference: anno,
		}
	}
}

impl<'l> AnnotatedLiteralRef<'l> {
	pub fn pre(&self) -> usize {
		self.reference.pre
	}
	pub fn post(&self) -> usize {
		self.reference.pre
	}
	pub fn as_str(&self) -> &str {
		self.reference.as_str()
	}
	pub fn len(&self) -> usize {
		self.reference.len
	}
}



#[derive(Clone,Debug)]
/// A litteral with meta info where the first and list whitespace may be found.
pub struct AnnotatedLiteral {
    /// The literal which this annotates to.
    pub literal: proc_macro2::Literal,
    /// the complete rendered string including post and pre.
    pub rendered: String,
    /// Whitespace prefix len + 1
    pub pre: usize,
    /// Whitespace postfix len + 1
    pub post: usize,
    /// Length without pre and post
    /// if all whitespace, this is zer0 and the sum of pre+post is 2x len
    pub len: usize,
}


impl std::cmp::PartialEq for AnnotatedLiteral {
	fn eq(&self, other: &Self) -> bool {
		if self.rendered != other.rendered {
			return false
		}
		if self.pre != other.pre {
			return false
		}
		if self.post != other.post {
			return false
		}
		if self.len != other.len {
			return false
		}
		if self.literal.span().start() != other.literal.span().start() {
			return false
		}
		if self.literal.span().end() != other.literal.span().end() {
			return false
		}

		true
	}
}

impl std::cmp::Eq for AnnotatedLiteral {}

impl From<proc_macro2::Literal> for AnnotatedLiteral {
    fn from(literal: proc_macro2::Literal) -> Self {
        let rendered = literal.to_string();
        let scrap = |c: &'_ char| -> bool { c.is_whitespace()};
        let pre = rendered.chars().take_while(scrap).count() + 1;
        let post = rendered.chars().rev().take_while(scrap).count() + 1;
        Self {
            len: if rendered.len() > pre + post {rendered.len() - pre - post} else { 0 },
            literal,
            rendered,
            pre,
            post,
        }
    }
}

impl std::ops::Deref for AnnotatedLiteral {
    type Target = proc_macro2::Literal;
    fn deref(&self) -> &Self::Target {
        &self.literal
    }
}

impl AnnotatedLiteral {
    pub fn as_str(&self) -> &str {
        &self.rendered.as_str()[self.pre..(self.pre+self.len)]
    }
}





#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct ConsecutiveLiteralSet {
    /// consecutive set of literals mapped by line number
    literals: Vec<AnnotatedLiteral>,
    /// lines spanned (start, end)
    pub coverage: (usize, usize),
}

impl ConsecutiveLiteralSet {
    /// Initiate a new set based on the first literal
    pub fn from(literal: AnnotatedLiteral) -> Self {
        Self {
            coverage: (literal.span().start().line, literal.span().end().line),
            literals: vec![literal],
        }
    }

    /// Add a literal to a literal set, if the previous lines literal already exists.
    ///
    /// Returns literl within the Err variant if not adjacent
    pub fn add_adjacent(
        &mut self,
        literal: AnnotatedLiteral,
    ) -> Result<(), AnnotatedLiteral> {
        let previous_line = literal.span().end().line;
        if previous_line == self.coverage.1 + 1 {
			self.coverage.1+=1;
            let _ = self.literals.push(literal);
            return Ok(());
        }

        let next_line = literal.span().start().line;
        if next_line + 1 == self.coverage.0 {
			let _ = self.literals.push(literal);
			self.coverage.1-=1;
            return Ok(());
        }

        Err(literal)
    }

    /// find the annotated which offset intersects
    ///
    /// returns a tuple of a literal that is intersected with offset
    /// and returns the relative offset within the stringified literal
    /// but also the `LineColumn` in a global context where to find it
    /// speaking of a global context.
    fn find_intersection<'a>(&self,
        mut offset: usize,
    ) -> Option<(Vec<&'a AnnotatedLiteral>, LineColumn, LineColumn)> {
        enum LookingFor {
            Start,
            End{ start: LineColumn },
        }

        let acc = Vec::with_capacity(8);
        let mut state = LookingFor::Start;
        let mut it = self.literals.iter();
        let mut opt = it.next();
        loop {
            let opt = if let Some(literal) = opt {
                // work on the string version length
                // such that we have the paddings removed
                // since this is what is sent to the checker
                state = match state {
                    LookingFor::Start => {
                        let len = literal.to_string().len();
                        if dbg!(offset) > dbg!(len) {
                            offset -= len;
                            LookingFor::Start
                            it.next()
                        } else {
                            LookingFor::End { start: LineColumn {
                                line: literal.span().start().line,
                                // add the padding again, to make for a sane global span
                                column: literal.span().start().column + offset + literal.pre,
                            }}
                            // do not advance the iterator, we need to check the same line for the end too!
                            opt
                        }

                    } ,

                    LookingFor::End { start } => {
                        if dbg!(offset) > dbg!(len) {
                            offset -= len;
                            LookingFor::End { start }
                        } else {
                            return Some((acc, start, end))
                        }
                    },

                }
                it.next()
            } else {
                break;
            }
        }
        None
    }

    /// Convert a linear offset to a set of offsets with literal references and spans within that literal.
    pub fn linear_coverage_to_spans<'a>(
        &'a self,
        offset: usize,
        length: usize,
    ) -> Vec<(&'a AnnotatedLiteral, Span)> {
        let mut x = self.literals.iter();

        trace!("linear coverage: offset={} length={} -> start {:?}", offset, length, start);
        if let Some((start_literal, start, mut remainder)) = dbg!(Self::find_intersection(&mut x, offset)) {
            let x2 = x.clone();
			trace!("linear coverage: remainder={} length={} -> start {:?}", remainder, length, start);
            remainder += length;
            if let Some((end_literal, end, _offset)) = dbg!(Self::find_intersection(&mut x, remainder)) {
                trace!("linear coverage: offset={} length{} -> end {:?}", offset, length, end);
                let start_post = start_literal.span().end().line;
                let end_pre = end_literal.span().start().line;
                if start_post < end_pre {
                    // calculate how many lines it spans
                    let n = end_pre - start_post + 1;
                    let mut acc = Vec::with_capacity(n);
                    // first literal to its end
                    acc.push((start_literal, Span { start, end: start_literal.span().end()}));

                    // take all in between completely
                    for literal in x2.skip(1).take(n-2) {
                        let span = Span {
                            start : literal.span().start(),
                            end: literal.span().end(),
                        };
                        acc.push((literal, span));
                    }
                    // add the last from the beginning to the computed end
                    acc.push((end_literal, Span { start: start_literal.span().start(), end}));
                    return acc
				} else {
                    assert_eq!(start_post, end_pre);
                    return vec![(start_literal, Span {
                        start,
                        end,
                    })]
                }

            }
		}
		trace!("linear coverage: offset={} length{} -> missing start or /end", offset, length);
        Vec::new()
    }

    pub fn literals<'x>(&'x self) -> Vec<&'x AnnotatedLiteral> {
        self.literals.iter().by_ref().collect()
	}

	pub fn len(&self) -> usize {
		self.literals.len()
	}
}

use std::fmt;

impl<'s> fmt::Display for ConsecutiveLiteralSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		let n = self.len();
        for literal in self.literals.iter().take(n - 1) {
            writeln!(formatter, "{}", literal.as_str())?;
		}
		if let Some(literal) = self.literals.last() {
			write!(formatter, "{}", literal.as_str())?;
		}
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

	const TEST: &str = r#"
	/// Another excellent verification pass.
	///
	/// Boats float, don't they?
	struct Vikings;
"#;

lazy_static::lazy_static!{
	static ref TEST_LITERALS: Vec<&'static str> = vec!["Another excellent verification pass.", "", "Boats float, don't they?"];
}

	const TEST_LITERALS_COMBINED: &str =
	r#"Another excellent verification pass.

Boats float, don't they?"#;


	fn literals() -> Vec<proc_macro2::Literal> {
		TEST_LITERALS.iter().enumerate().map(|(idx,x)| {
			let lit = proc_macro2::Literal::string(x);
			lit
		}).collect()
	}

	fn annotated_literals() -> Vec<AnnotatedLiteral> {
		literals().into_iter().map(|literal| {
			AnnotatedLiteral::from(literal)
		}).collect()
	}

    #[test]
    #[ignore = "can not succeed, since all spans are (line=0,column=0)"]
	fn combine_literals() {
		let literals = annotated_literals();

		let mut cls = ConsecutiveLiteralSet::default();
		for literal in literals {
			assert!(cls.add_adjacent(literal).is_ok());
		}

		assert_eq!(cls.to_string(), TEST_LITERALS_COMBINED.to_string());
	}
}