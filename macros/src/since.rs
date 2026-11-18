use std::str::FromStr;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::quote;
use syn::parse::Parser;
use syn::spanned::Spanned;

use crate::utils;

pub struct Since {
    pub(crate) version: Option<String>,
    pub(crate) date: Option<String>,
}

impl Since {
    /// Build a `Since` struct from the given attribute arguments.
    pub(crate) fn new(args: TokenStream) -> Result<Since, syn::Error> {
        let mut since = Since {
            version: None,
            date: None,
        };

        type AttributeArgs = syn::punctuated::Punctuated<syn::Meta, syn::Token![,]>;

        let parsed_args = AttributeArgs::parse_terminated.parse(args.clone())?;

        for arg in parsed_args {
            match arg {
                syn::Meta::NameValue(namevalue) => {
                    let q = namevalue
                        .path
                        .get_ident()
                        .ok_or_else(|| syn::Error::new_spanned(&namevalue, "Must have specified ident"))?;

                    let ident = q.to_string().to_lowercase();

                    match ident.as_str() {
                        "version" => {
                            since.set_version(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        "date" => {
                            since.set_date(namevalue.value.clone(), Spanned::span(&namevalue.value))?;
                        }

                        name => {
                            let msg = format!(
                                "Unknown attribute {} is specified; expected one of: `version`, `date`",
                                name,
                            );
                            return Err(syn::Error::new_spanned(namevalue, msg));
                        }
                    }
                }
                other => {
                    return Err(syn::Error::new_spanned(other, "Unknown attribute inside the macro"));
                }
            }
        }

        if since.version.is_none() {
            return Err(syn::Error::new_spanned(
                proc_macro2::TokenStream::from(args),
                "Missing `version` attribute",
            ));
        }

        Ok(since)
    }
    /// Append a `since` doc such as `Since: 1.0.0` to the bottom of the doc section.
    pub(crate) fn append_since_doc(self, item: TokenStream) -> Result<TokenStream, syn::Error> {
        let item = proc_macro2::TokenStream::from(item);

        // Present docs to skip, in order to append `since` at bottom of doc section.
        let mut present_docs = vec![];

        // Tokens left after present docs.
        let mut last_non_doc = vec![];

        let mut it = item.clone().into_iter();
        loop {
            let Some(curr) = it.next() else {
                break;
            };
            let Some(next) = it.next() else {
                last_non_doc.push(curr);
                break;
            };

            if utils::is_doc(&curr, &next) {
                present_docs.push(curr);
                present_docs.push(next);
            } else {
                last_non_doc.push(curr);
                last_non_doc.push(next);
                break;
            }
        }

        let since_docs_str = if present_docs.is_empty() {
            format!(r#"#[doc = " {}"]"#, self.to_doc_string())
        } else {
            // If there are already docs, insert a blank line.
            format!(r#"#[doc = ""] #[doc = " {}"]"#, self.to_doc_string())
        };
        let since_docs = proc_macro2::TokenStream::from_str(&since_docs_str).unwrap();

        let present_docs: proc_macro2::TokenStream = present_docs.into_iter().collect();
        let last_non_docs: proc_macro2::TokenStream = last_non_doc.into_iter().collect();

        // Other non doc tokens.
        let other: proc_macro2::TokenStream = it.collect();

        let tokens = quote! {
            #present_docs
            #since_docs
            #last_non_docs
            #other
        };
        Ok(tokens.into())
    }

    /// Build doc string: `Since: 1.0.0, Date(2021-01-01)` or  `Since: 1.0.0`
    fn to_doc_string(&self) -> String {
        let mut s = String::new();
        s.push_str("Since: ");

        if let Some(version) = &self.version {
            s.push_str(version.as_str());
        }
        if let Some(date) = &self.date {
            s.push_str(&format!(", Date({})", date));
        }
        s
    }

    pub(crate) fn set_version(&mut self, ver_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        if self.version.is_some() {
            return Err(syn::Error::new(span, "`version` set multiple times."));
        }

        let ver = Self::parse_str(ver_lit, "version", span)?;

        semver::Version::parse(&ver)
            .map_err(|_e| syn::Error::new(span, format!("`version`(`{}`) is not valid semver.", ver)))?;

        self.version = Some(ver);

        Ok(())
    }

    pub(crate) fn set_date(&mut self, date_lit: syn::Expr, span: Span) -> Result<(), syn::Error> {
        if self.date.is_some() {
            return Err(syn::Error::new(span, "`date` set multiple times."));
        }

        let date_str = Self::parse_str(date_lit, "date", span)?;

        if !is_valid_ymd_date(&date_str) {
            return Err(syn::Error::new(
                span,
                format!(
                    "`date`(`{}`) is not valid date string. Expected format: yyyy-mm-dd",
                    date_str
                ),
            ));
        }

        self.date = Some(date_str);

        Ok(())
    }

    /// Extract string from `foo` or `"foo"`
    fn parse_str(expr: syn::Expr, field: &str, span: Span) -> Result<String, syn::Error> {
        let s = match expr {
            syn::Expr::Lit(s) => match s.lit {
                syn::Lit::Str(s) => s.value(),
                syn::Lit::Verbatim(s) => s.to_string(),
                _ => {
                    return Err(syn::Error::new(
                        span,
                        format!("Failed to parse value of `{}` as string.", field),
                    ))
                }
            },
            syn::Expr::Verbatim(s) => s.to_string(),
            _ => {
                return Err(syn::Error::new(
                    span,
                    format!("Failed to parse value of `{}` as string.", field),
                ))
            }
        };
        Ok(s)
    }
}

/// Smallest year accepted by chrono 0.4's `NaiveDate`.
const MIN_YEAR: i64 = -262_143;
/// Largest year accepted by chrono 0.4's `NaiveDate`.
const MAX_YEAR: i64 = 262_142;

/// Returns whether `s` is a valid `%Y-%m-%d` date.
///
/// Accepts exactly what `chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")`
/// (chrono 0.4) accepted, which this macro used before:
/// - whitespace is skipped before each number;
/// - the year is 1 to 4 digits, or any number of digits after an explicit `+` or `-` sign;
/// - month and day are 1 or 2 digits;
/// - no trailing input;
/// - the date exists in the proleptic Gregorian calendar, with the year in chrono's range.
fn is_valid_ymd_date(s: &str) -> bool {
    let Some((year, month, day)) = parse_ymd(s) else {
        return false;
    };
    if !(MIN_YEAR..=MAX_YEAR).contains(&year) {
        return false;
    }
    let (Ok(month), Ok(day)) = (i8::try_from(month), i8::try_from(day)) else {
        return false;
    };
    // The Gregorian calendar repeats every 400 years, so validating the same
    // month and day in an equivalent year inside jiff's range (-9999..=9999)
    // is exact for every year in chrono's range.
    let equivalent_year = 2000 + year.rem_euclid(400);
    let equivalent_year = i16::try_from(equivalent_year).expect("2000..2400 fits in i16");
    jiff::civil::Date::new(equivalent_year, month, day).is_ok()
}

/// Splits `s` into `(year, month, day)` following chrono's `%Y-%m-%d` parser.
fn parse_ymd(s: &str) -> Option<(i64, i64, i64)> {
    let (year, s) = parse_year(s.trim_start())?;
    let s = s.strip_prefix('-')?;
    let (month, s) = parse_digits(s.trim_start(), 2)?;
    let s = s.strip_prefix('-')?;
    let (day, s) = parse_digits(s.trim_start(), 2)?;
    if !s.is_empty() {
        return None;
    }
    Some((year, month, day))
}

/// Parses chrono's `%Y`: an explicit sign allows any number of digits,
/// otherwise at most 4 digits are consumed.
fn parse_year(s: &str) -> Option<(i64, &str)> {
    if let Some(rest) = s.strip_prefix('-') {
        let (v, rest) = parse_digits(rest, usize::MAX)?;
        Some((v.checked_neg()?, rest))
    } else if let Some(rest) = s.strip_prefix('+') {
        parse_digits(rest, usize::MAX)
    } else {
        parse_digits(s, 4)
    }
}

/// Consumes 1 to `max` leading ASCII digits; `None` if there are none or on overflow.
fn parse_digits(s: &str, max: usize) -> Option<(i64, &str)> {
    let len = s.bytes().take(max).take_while(u8::is_ascii_digit).count();
    if len == 0 {
        return None;
    }
    let mut v: i64 = 0;
    for b in s[..len].bytes() {
        v = v.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
    }
    Some((v, &s[len..]))
}

#[cfg(test)]
mod tests {
    use super::is_valid_ymd_date;

    #[test]
    fn test_valid_dates() {
        for s in [
            "2021-01-01",
            "2024-02-29",
            "2000-02-29",
            "1999-12-31",
            "0000-01-01",
            "1-1-1",
            "21-1-01",
            " 2021-01-01",
            "2021- 1- 1",
            "2021-\t01-01",
            "+2021-01-01",
            "+0002021-01-01",
            "-2021-01-01",
            "+262142-12-31",
            "-262143-01-01",
            "-0004-02-29",
        ] {
            assert!(is_valid_ymd_date(s), "expected valid: {:?}", s);
        }
    }

    #[test]
    fn test_invalid_dates() {
        for s in [
            "2021-01--",
            "",
            "2021",
            "2021-01",
            "2021-01-",
            "2021-13-01",
            "2021-00-01",
            "2021-01-00",
            "2021-01-32",
            "2023-02-29",
            "1900-02-29",
            "2021-04-31",
            "20210-01-01",
            "2021-001-01",
            "2021-01-001",
            "2021 -01-01",
            "2021-01-01 ",
            "2021/01/01",
            "+262143-01-01",
            "-262144-12-31",
            "+99999999999999999999-01-01",
            "- 2021-01-01",
            "2021-+1-01",
            "yyyy-mm-dd",
        ] {
            assert!(!is_valid_ymd_date(s), "expected invalid: {:?}", s);
        }
    }
}
