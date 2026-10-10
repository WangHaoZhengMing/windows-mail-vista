//! A small static-mail CSS dialect shared by engines. MSHTML cannot rely on CSP.
use super::ImageUrls;
use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, ToCss, Token,
};

struct StaticCss<'a> {
    urls: &'a ImageUrls,
    depth: usize,
}

pub(super) fn declarations(css: &str, urls: &ImageUrls) -> String {
    declaration_list(&mut Parser::new(css), urls)
}
fn declaration_list(input: &mut Parser<'_>, urls: &ImageUrls) -> String {
    RuleBodyParser::new(input, &mut StaticCss { urls, depth: 0 })
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
        .join(";")
}
pub(super) fn stylesheet(css: &str, urls: &ImageUrls) -> String {
    rule_list(&mut Parser::new(css), urls, 0)
        // CSS strings are decoded by the lexer. Do not synthesize an HTML
        // closing style tag from a formerly escaped '<'.
        .replace('<', "\\3c ")
}
fn rule_list(input: &mut Parser<'_>, urls: &ImageUrls, depth: usize) -> String {
    StyleSheetParser::new(input, &mut StaticCss { urls, depth })
        .filter_map(Result::ok)
        .collect::<Vec<_>>()
        .join("")
}

impl<'i> DeclarationParser<'i> for StaticCss<'_> {
    type Declaration = String;
    type Error = ();
    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _: &ParserState,
    ) -> Result<String, ParseError<()>> {
        let name = name.to_ascii_lowercase();
        if !static_property(&name) {
            return Err(ParseError::unexpected_token());
        }
        let value = value_tokens(input, self.urls, 0)?;
        Ok(format!("{name}:{}", value.trim()))
    }
}
impl<'i> AtRuleParser<'i> for StaticCss<'_> {
    type Prelude = String;
    type AtRule = String;
    type Error = ();
    // Preserve responsive media rules; imports and downloadable fonts fail.
    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<String, ParseError<()>> {
        if !name.eq_ignore_ascii_case("media") || self.depth >= 16 {
            return Err(ParseError::unexpected_token());
        }
        let start = input.position();
        while let Ok(token) = input.next_including_whitespace() {
            if token.is_parse_error()
                || matches!(token, Token::AtKeyword(_) | Token::UnquotedUrl(_))
            {
                return Err(ParseError::unexpected_token());
            }
        }
        Ok(input.slice_from(start).to_owned())
    }
    fn parse_block(
        &mut self,
        condition: String,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<String, ParseError<()>> {
        Ok(format!(
            "@media {condition}{{{}}}",
            rule_list(input, self.urls, self.depth + 1)
        ))
    }
}
impl<'i> QualifiedRuleParser<'i> for StaticCss<'_> {
    type Prelude = String;
    type QualifiedRule = String;
    type Error = ();
    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<String, ParseError<()>> {
        let start = input.position();
        while let Ok(token) = input.next_including_whitespace() {
            if token.is_parse_error()
                || matches!(token, Token::AtKeyword(_) | Token::UnquotedUrl(_))
            {
                return Err(ParseError::unexpected_token());
            }
        }
        let selector = input.slice_from(start).trim();
        if selector.is_empty() {
            return Err(ParseError::unexpected_token());
        }
        Ok(selector.to_owned())
    }
    fn parse_block(
        &mut self,
        selector: String,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<String, ParseError<()>> {
        Ok(format!(
            "{selector}{{{}}}",
            declaration_list(input, self.urls)
        ))
    }
}
impl<'i> RuleBodyItemParser<'i, String, ()> for StaticCss<'_> {
    fn parse_declarations(&self) -> bool {
        true
    }
    fn parse_qualified(&self) -> bool {
        false
    }
}

fn value_tokens<'i>(
    input: &mut Parser<'i>,
    urls: &ImageUrls,
    depth: usize,
) -> Result<String, ParseError<()>> {
    if depth > 16 {
        return Err(ParseError::unexpected_token());
    }
    let mut output = String::new();
    while let Ok(token) = input.next_including_whitespace().cloned() {
        if token.is_parse_error() {
            return Err(ParseError::unexpected_token());
        }
        match token {
            Token::Comment(_) => output.push(' '),
            Token::UnquotedUrl(url) => append_image(&mut output, &url, urls)?,
            Token::Function(name) if name.eq_ignore_ascii_case("url") => {
                let url = input.parse_nested_block(|nested| {
                    let value = nested.expect_string()?.to_string();
                    nested.expect_exhausted()?;
                    Ok::<_, ParseError<()>>(value)
                })?;
                append_image(&mut output, &url, urls)?;
            }
            Token::Function(name) if static_function(&name) => {
                let contents =
                    input.parse_nested_block(|nested| value_tokens(nested, urls, depth + 1))?;
                output.push_str(&name.to_ascii_lowercase());
                output.push('(');
                output.push_str(&contents);
                output.push(')');
            }
            Token::ParenthesisBlock => {
                let contents =
                    input.parse_nested_block(|nested| value_tokens(nested, urls, depth + 1))?;
                output.push('(');
                output.push_str(&contents);
                output.push(')');
            }
            Token::Function(_)
            | Token::AtKeyword(_)
            | Token::CurlyBracketBlock
            | Token::SquareBracketBlock
            | Token::Semicolon => return Err(ParseError::unexpected_token()),
            token => output.push_str(&token.to_css_string()),
        }
    }
    Ok(output)
}
fn append_image(output: &mut String, url: &str, urls: &ImageUrls) -> Result<(), ParseError<()>> {
    let Some(url) = urls.resolve(url) else {
        return Err(ParseError::unexpected_token());
    };
    output.push_str("url(");
    output.push_str(&Token::QuotedString(url.into()).to_css_string());
    output.push(')');
    Ok(())
}
fn static_function(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "rgb"
            | "rgba"
            | "hsl"
            | "hsla"
            | "calc"
            | "min"
            | "max"
            | "clamp"
            | "linear-gradient"
            | "radial-gradient"
            | "repeating-linear-gradient"
            | "repeating-radial-gradient"
    )
}
fn static_property(name: &str) -> bool {
    matches!(
        name,
        "background"
            | "background-color"
            | "background-image"
            | "background-position"
            | "background-repeat"
            | "background-size"
            | "background-attachment"
            | "color"
            | "font"
            | "font-family"
            | "font-size"
            | "font-style"
            | "font-weight"
            | "font-variant"
            | "line-height"
            | "letter-spacing"
            | "word-spacing"
            | "text-align"
            | "text-decoration"
            | "text-transform"
            | "text-indent"
            | "text-shadow"
            | "white-space"
            | "word-wrap"
            | "overflow-wrap"
            | "word-break"
            | "vertical-align"
            | "margin"
            | "margin-left"
            | "margin-right"
            | "margin-top"
            | "margin-bottom"
            | "padding"
            | "padding-left"
            | "padding-right"
            | "padding-top"
            | "padding-bottom"
            | "border"
            | "border-left"
            | "border-right"
            | "border-top"
            | "border-bottom"
            | "border-width"
            | "border-color"
            | "border-style"
            | "border-collapse"
            | "border-spacing"
            | "border-radius"
            | "width"
            | "height"
            | "min-width"
            | "max-width"
            | "min-height"
            | "max-height"
            | "table-layout"
            | "caption-side"
            | "empty-cells"
            | "box-sizing"
            | "display"
            | "visibility"
            | "overflow"
            | "overflow-x"
            | "overflow-y"
            | "float"
            | "clear"
            | "position"
            | "top"
            | "right"
            | "bottom"
            | "left"
            | "z-index"
            | "opacity"
            | "list-style"
            | "list-style-type"
            | "list-style-position"
            | "list-style-image"
            | "direction"
            | "unicode-bidi"
    )
}
