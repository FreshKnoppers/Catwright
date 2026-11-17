//! Tiny, safe arithmetic evaluator for the `computed` UI node.
//!
//! The model may put a formula into a ```ui block; it is evaluated locally so
//! the result follows the sliders/fields of the same block without asking the
//! model again. There is no `eval` of model code: a recursive-descent parser
//! over a fixed grammar, with no access to anything but the given variables.
//!
//! Grammar (lowest to highest precedence):
//! ```text
//! expr    := term (('+' | '-') term)*
//! term    := unary (('*' | '/' | '%') unary)*
//! unary   := ('-' | '+') unary | power
//! power   := primary ('^' unary)?          // right associative
//! primary := number | ident | ident '(' args ')' | '(' expr ')'
//! args    := expr (',' expr)*
//! ```

use crate::i18n::{tr, trf};
use std::collections::HashMap;

/// Longest accepted expression (characters).
const MAX_LEN: usize = 500;
/// Deepest accepted nesting (parentheses, function calls, unary chains).
const MAX_DEPTH: usize = 32;

/// Evaluate `expr` with the given variables. Errors are German one-liners
/// meant to be shown in the UI instead of a result.
pub fn eval(expr: &str, vars: &HashMap<String, f64>) -> Result<f64, String> {
    if expr.trim().is_empty() {
        return Err(tr("Empty expression.").to_string());
    }
    if expr.chars().count() > MAX_LEN {
        return Err(trf(
            "Expression too long (max. {max} characters).",
            &[("max", &MAX_LEN)],
        ));
    }
    let tokens = tokenize(expr)?;
    let mut p = Parser {
        tokens,
        pos: 0,
        vars,
        depth: 0,
    };
    let value = p.expr()?;
    if p.pos != p.tokens.len() {
        return Err(tr("Unexpected character in the expression.").to_string());
    }
    if !value.is_finite() {
        return Err(tr("The result is not a finite number.").to_string());
    }
    Ok(value)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            // exponent part: 1e3, 2.5E-2
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                let mut j = i + 1;
                if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                    j += 1;
                }
                if j < chars.len() && chars[j].is_ascii_digit() {
                    i = j;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
            }
            let text: String = chars[start..i].iter().collect();
            let value = text
                .parse::<f64>()
                .map_err(|_| trf("Invalid number: {text}", &[("text", &text)]))?;
            out.push(Tok::Num(value));
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            out.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        let tok = match c {
            '+' => Tok::Plus,
            '-' => Tok::Minus,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '%' => Tok::Percent,
            '^' => Tok::Caret,
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            ',' => Tok::Comma,
            other => return Err(trf("Unexpected character “{char}”.", &[("char", &other)])),
        };
        out.push(tok);
        i += 1;
    }
    Ok(out)
}

struct Parser<'a> {
    tokens: Vec<Tok>,
    pos: usize,
    vars: &'a HashMap<String, f64>,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn enter(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(trf(
                "Expression nested too deeply (max. {max}).",
                &[("max", &MAX_DEPTH)],
            ));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn expr(&mut self) -> Result<f64, String> {
        self.enter()?;
        let mut value = self.term()?;
        loop {
            match self.peek() {
                Some(Tok::Plus) => {
                    self.next();
                    value += self.term()?;
                }
                Some(Tok::Minus) => {
                    self.next();
                    value -= self.term()?;
                }
                _ => break,
            }
        }
        self.leave();
        Ok(value)
    }

    fn term(&mut self) -> Result<f64, String> {
        let mut value = self.unary()?;
        loop {
            match self.peek() {
                Some(Tok::Star) => {
                    self.next();
                    value *= self.unary()?;
                }
                Some(Tok::Slash) => {
                    self.next();
                    let rhs = self.unary()?;
                    if rhs == 0.0 {
                        return Err(tr("Division by 0.").to_string());
                    }
                    value /= rhs;
                }
                Some(Tok::Percent) => {
                    self.next();
                    let rhs = self.unary()?;
                    if rhs == 0.0 {
                        return Err(tr("Division by 0.").to_string());
                    }
                    value %= rhs;
                }
                _ => break,
            }
        }
        Ok(value)
    }

    fn unary(&mut self) -> Result<f64, String> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.next();
                self.enter()?;
                let v = self.unary()?;
                self.leave();
                Ok(-v)
            }
            Some(Tok::Plus) => {
                self.next();
                self.enter()?;
                let v = self.unary()?;
                self.leave();
                Ok(v)
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if matches!(self.peek(), Some(Tok::Caret)) {
            self.next();
            // right associative: 2^3^2 = 2^(3^2)
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Tok::Num(v)) => Ok(v),
            Some(Tok::LParen) => {
                self.enter()?;
                let v = self.expr()?;
                self.leave();
                match self.next() {
                    Some(Tok::RParen) => Ok(v),
                    _ => Err(tr("Missing closing parenthesis.").to_string()),
                }
            }
            Some(Tok::Ident(name)) => {
                if matches!(self.peek(), Some(Tok::LParen)) {
                    self.next();
                    self.enter()?;
                    let mut args = Vec::new();
                    if matches!(self.peek(), Some(Tok::RParen)) {
                        self.next();
                    } else {
                        loop {
                            args.push(self.expr()?);
                            match self.next() {
                                Some(Tok::Comma) => continue,
                                Some(Tok::RParen) => break,
                                _ => return Err(tr("Missing closing parenthesis.").to_string()),
                            }
                        }
                    }
                    self.leave();
                    call(&name, &args)
                } else {
                    self.vars
                        .get(&name)
                        .copied()
                        .ok_or_else(|| trf("Unknown variable “{name}”.", &[("name", &name)]))
                }
            }
            Some(_) => Err(tr("Unexpected character in the expression.").to_string()),
            None => Err(tr("Incomplete expression.").to_string()),
        }
    }
}

fn call(name: &str, args: &[f64]) -> Result<f64, String> {
    let arity = |n: usize| -> Result<(), String> {
        if args.len() == n {
            Ok(())
        } else {
            Err(trf(
                "{name} expects {n} argument(s).",
                &[("name", &name), ("n", &n)],
            ))
        }
    };
    match name {
        "min" | "max" => {
            if args.len() < 2 {
                return Err(trf(
                    "{name} expects at least 2 arguments.",
                    &[("name", &name)],
                ));
            }
            let mut acc = args[0];
            for v in &args[1..] {
                acc = if name == "min" {
                    acc.min(*v)
                } else {
                    acc.max(*v)
                };
            }
            Ok(acc)
        }
        "round" => {
            arity(1)?;
            Ok(args[0].round())
        }
        "floor" => {
            arity(1)?;
            Ok(args[0].floor())
        }
        "ceil" => {
            arity(1)?;
            Ok(args[0].ceil())
        }
        "abs" => {
            arity(1)?;
            Ok(args[0].abs())
        }
        "sqrt" => {
            arity(1)?;
            if args[0] < 0.0 {
                return Err(tr("sqrt of a negative number.").to_string());
            }
            Ok(args[0].sqrt())
        }
        other => Err(trf("Unknown function “{name}”.", &[("name", &other)])),
    }
}

#[cfg(test)]
mod tests {
    use super::eval;
    use std::collections::HashMap;

    fn vars() -> HashMap<String, f64> {
        let mut m = HashMap::new();
        m.insert("a".to_string(), 2.0);
        m.insert("b".to_string(), 3.0);
        m.insert("rate".to_string(), 0.05);
        m
    }

    fn ok(expr: &str) -> f64 {
        eval(expr, &vars()).expect("expression evaluates")
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(ok("2+3*4"), 14.0);
        assert_eq!(ok("2*3+4"), 10.0);
        assert_eq!(ok("10-2-3"), 5.0);
        assert_eq!(ok("2^3^2"), 512.0, "power is right associative");
        assert_eq!(ok("2*3^2"), 18.0, "power binds tighter than *");
        assert_eq!(ok("7%4"), 3.0);
        assert_eq!(ok("8/2/2"), 2.0);
    }

    #[test]
    fn parentheses_and_unary_minus() {
        assert_eq!(ok("(2+3)*4"), 20.0);
        assert_eq!(ok("2*(3+(4-1))"), 12.0);
        assert_eq!(ok("-3+5"), 2.0);
        assert_eq!(ok("-(2+3)"), -5.0);
        assert_eq!(ok("--4"), 4.0);
        assert_eq!(ok("2*-3"), -6.0);
        assert_eq!(ok("+7"), 7.0);
    }

    #[test]
    fn numbers_with_decimals_and_exponents() {
        assert_eq!(ok("1.5*2"), 3.0);
        assert_eq!(ok("1e3"), 1000.0);
        assert_eq!(ok("2.5E-1*10"), 2.5);
        assert_eq!(ok(".5+.5"), 1.0);
    }

    #[test]
    fn variables() {
        assert_eq!(ok("a*b"), 6.0);
        assert_eq!(ok("a + b * 2"), 8.0);
        assert_eq!(ok("rate*100"), 5.0);
    }

    #[test]
    fn functions() {
        assert_eq!(ok("min(3,1,2)"), 1.0);
        assert_eq!(ok("max(3,1,2)"), 3.0);
        assert_eq!(ok("round(2.6)"), 3.0);
        assert_eq!(ok("floor(2.9)"), 2.0);
        assert_eq!(ok("ceil(2.1)"), 3.0);
        assert_eq!(ok("abs(-4)"), 4.0);
        assert_eq!(ok("sqrt(9)"), 3.0);
        assert_eq!(ok("max(a,b)+min(a,b)"), 5.0);
        assert_eq!(ok("round(sqrt(2)*10)/10"), 1.4);
    }

    #[test]
    fn errors_are_descriptive() {
        let e = eval("1/0", &vars()).unwrap_err();
        assert!(e.contains("Division by 0"), "{e}");
        let e = eval("1%0", &vars()).unwrap_err();
        assert!(e.contains("Division by 0"), "{e}");
        let e = eval("x+1", &vars()).unwrap_err();
        assert!(e.contains("Unknown variable"), "{e}");
        let e = eval("2+", &vars()).unwrap_err();
        assert!(e.contains("Incomplete"), "{e}");
        let e = eval("2+*3", &vars()).unwrap_err();
        assert!(e.contains("Unexpected character"), "{e}");
        let e = eval("(1+2", &vars()).unwrap_err();
        assert!(e.contains("parenthesis"), "{e}");
        let e = eval("1 2", &vars()).unwrap_err();
        assert!(e.contains("Unexpected character"), "{e}");
        let e = eval("nope(1)", &vars()).unwrap_err();
        assert!(e.contains("Unknown function"), "{e}");
        let e = eval("min(1)", &vars()).unwrap_err();
        assert!(e.contains("at least 2"), "{e}");
        let e = eval("sqrt(-1)", &vars()).unwrap_err();
        assert!(e.contains("sqrt"), "{e}");
        let e = eval("", &vars()).unwrap_err();
        assert!(e.contains("Empty expression"), "{e}");
        let e = eval("1$2", &vars()).unwrap_err();
        assert!(e.contains("Unexpected character"), "{e}");
    }

    #[test]
    fn nesting_and_length_limits() {
        let deep = format!("{}1{}", "(".repeat(40), ")".repeat(40));
        let e = eval(&deep, &vars()).unwrap_err();
        assert!(e.contains("nested too deeply"), "{e}");
        // 10 levels are still fine
        let ok_deep = format!("{}1{}", "(".repeat(10), ")".repeat(10));
        assert_eq!(eval(&ok_deep, &vars()).unwrap(), 1.0);
        let long = "1+".repeat(300) + "1";
        let e = eval(&long, &vars()).unwrap_err();
        assert!(e.contains("too long"), "{e}");
    }

    #[test]
    fn no_access_outside_vars() {
        // identifiers are only looked up in `vars`; nothing else is reachable
        let empty = HashMap::new();
        assert!(eval("std", &empty).is_err());
        assert!(eval("env", &empty).is_err());
        assert!(eval("a", &empty).is_err());
    }
}
