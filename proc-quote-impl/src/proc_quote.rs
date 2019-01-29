use proc_macro2::*;
use quote::{quote, quote_spanned, TokenStreamExt};

struct Error {
    span_s: Span,
    span_e: Option<Span>,
    msg: &'static str,
}

impl Error {
    fn new(span: Span, msg: &'static str) -> Self {
        Self { span_s: span, span_e: None, msg }
    }

    fn end_span(mut self, span: Span) -> Self {
        self.span_e = Some(span);
        self
    }

    fn raise(self) -> TokenStream {
        let Error{span_s, span_e, msg} = self;

        let compile_error = quote_spanned!{ span_s=>
            compile_error!(#msg)
        };

        quote_spanned!{ span_e.unwrap_or(span_s)=>
            #compile_error ;
        }
    }
}

type Result<T> = std::result::Result<T, Error>; 

/// Wraps the inner content inside a block with boilerplate to create and return `__stream`.
fn generate_quote_header(inner: TokenStream) -> TokenStream {
    quote! {
        {
            let mut __stream = ::proc_quote::__rt::TokenStream::new();
            #inner
            __stream
        }
    }
}

/// Transforms an `Ident` into code that appends the given `Ident` into `__stream`.
fn parse_ident(stream: &mut TokenStream, ident: &Ident) {
    let ref_mut_stream = quote!{ &mut __stream };
    let span = ident.span();
    let ident = ident.to_string();
    stream.append_all(quote_spanned! { span=>
        ::proc_quote::__rt::append_ident(#ref_mut_stream, #ident, ::proc_quote::__rt::Span::call_site());
    });
}

/// Transforms a `Punct` into code that appends the given `Punct` into `__stream`.
fn parse_punct(stream: &mut TokenStream, punct: &Punct) {
    let ref_mut_stream = quote!{ &mut __stream };
    let span = punct.span();
    let spacing = punct.spacing();
    let punct = punct.as_char();
    let append = match spacing {
        Spacing::Alone => quote_spanned! { span=>
            ::proc_quote::__rt::append_punct(#ref_mut_stream, #punct, ::proc_quote::__rt::Spacing::Alone);
        },
        Spacing::Joint => quote_spanned! { span=>
            ::proc_quote::__rt::append_punct(#ref_mut_stream, #punct, ::proc_quote::__rt::Spacing::Joint);
        },
    };
    stream.append_all(append);
}

/// Transforms a `Literal` into code that appends the given `Literal` into `__stream`.
fn parse_literal(stream: &mut TokenStream, lit: &Literal) {
    let ref_mut_stream = quote!{ &mut __stream };
    let span = lit.span();
    let lit_to_string = lit.to_string();

    if [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "f32", "f64", "\"", "\'", "#",
    ]
    .iter()
    .any(|suffix| lit_to_string.ends_with(suffix))
    {
        // Number with a suffix, char, str, raw char, raw str
        // It should be safe to turn them into tokens
        stream.append_all(quote_spanned! { span=>
            ::proc_quote::__rt::append_to_tokens(#ref_mut_stream, & #lit);
        });
    } else {
        // Integer without suffix, float without suffix
        // Must be more careful, in order for the macro not to assume a wrong suffix
        if let Ok(i) = lit_to_string.parse::<i32>() {
            stream.append_all(quote_spanned! { span=>
                ::proc_quote::__rt::append_lit(#ref_mut_stream, Literal::i32_unsuffixed(#i));
            });
        } else if let Ok(i) = lit_to_string.parse::<i64>() {
            stream.append_all(quote_spanned! { span=>
                ::proc_quote::__rt::append_lit(#ref_mut_stream, Literal::i64_unsuffixed(#i));
            });
        } else if let Ok(u) = lit_to_string.parse::<u64>() {
            stream.append_all(quote_spanned! { span=>
                ::proc_quote::__rt::append_lit(#ref_mut_stream, Literal::u64_unsuffixed(#u));
            });
        } else if let Ok(f) = lit_to_string.parse::<f64>() {
            stream.append_all(quote_spanned! { span=>
                ::proc_quote::__rt::append_lit(#ref_mut_stream, Literal::f64_unsuffixed(#f));
            });
        } else {
            // This should never show up
            panic!("Unable to parse this literal. Please, fill in an issue in `proc-macro`'s repository.");
        }
    }
}

/// Logic common to `parse_group` and `parse_group_in_iterator_pattern`.
fn parse_group_inner(stream: &mut TokenStream, inner: TokenStream, delimiter: Delimiter, group_span: Span) {
    let ref_mut_stream = quote!{ &mut __stream };
    let delimiter = match delimiter {
        Delimiter::Parenthesis => quote! {
            ::proc_quote::__rt::Delimiter::Parenthesis
        },
        Delimiter::Brace => quote! {
            ::proc_quote::__rt::Delimiter::Brace
        },
        Delimiter::Bracket => quote! {
            ::proc_quote::__rt::Delimiter::Bracket
        },
        Delimiter::None => quote! {
            ::proc_quote::__rt::Delimiter::None
        },
    };

    stream.append_all(quote_spanned! { group_span =>
        ::proc_quote::__rt::append_group(#ref_mut_stream, #inner, #delimiter);
    });
}

/// Transforms a `Group` into code that appends the given `Group` into `__stream`.
///
/// Inside iterator patterns, use `parse_group_in_iterator_pattern`.
fn parse_group(stream: &mut TokenStream, group: &Group) -> Result<()> {
    let inner = parse_token_stream(group.stream())?;
    let inner = generate_quote_header(inner);

    Ok(parse_group_inner(stream, inner, group.delimiter(), group.span()))
}

/// Transforms a `Group` into code that appends the given `Group` into `__stream`.
///
/// This function is used inside the iterator patterns, to check for iterators used
/// inside.
fn parse_group_in_iterator_pattern(
    stream: &mut TokenStream,
    group: &Group,
    iter_idents: &mut Vec<Ident>,
) -> Result<()> {
    let inner = parse_token_stream_in_iterator_pattern(group.stream(), iter_idents)?;
    let inner = generate_quote_header(inner);

    Ok(parse_group_inner(stream, inner, group.delimiter(), group.span()))
}

/// Helper enum for `interpolation_pattern_type`'s return type.
enum InterpolationPattern {
    /// #ident
    Ident(Ident),

    /// #( group ) token_stream *
    Iterator(Group, TokenStream),

    /// Not an interpolation pattern
    None,
}

/// Helper type alias for `interpolation_pattern_type`'s input type.
type InputIter = std::iter::Peekable<token_stream::IntoIter>;

/// Returns the interpolation pattern type based on the content of the given 
/// `punct` and the rest of the `input`.
/// 
/// Input that is part of the pattern is automatically consumed.
fn interpolation_pattern_type(
    punct: &Punct,
    input: &mut InputIter,
) -> Result<InterpolationPattern> {
    match (punct.as_char(), input.peek()) {
        // #ident
        ('#', Some(TokenTree::Ident(_))) => {
            if let Some(TokenTree::Ident(ident)) = input.next() {
                Ok(InterpolationPattern::Ident(ident))
            } else {
                panic!("guaranteed by previous match")
            }
        },

        // #(group)
        ('#', Some(TokenTree::Group(group))) if group.delimiter() == Delimiter::Parenthesis => {
            let inner = match input.next() {
                Some(TokenTree::Group(inner)) => inner,
                _ => panic!("guaranteed by previous match"),   
            };

            let separator = parse_separator(input, inner.span())?;

            Ok(InterpolationPattern::Iterator(inner, separator))
        },

        // Not an interpolation pattern
        _ => Ok(InterpolationPattern::None),
    }
}

/// Interpolates the given variable, which should implement `ToTokens`.
fn interpolate_to_tokens_ident(stream: &mut TokenStream, ident: &Ident) {
    let ref_mut_stream = quote!{ &mut __stream };
    let span = ident.span();
    stream.append_all(quote_spanned! { span=>
        ::proc_quote::__rt::append_to_tokens(#ref_mut_stream, & #ident);
    });
}

/// Interpolates the expression inside the group, which should evaluate to
/// something that implements `ToTokens`.
fn interpolate_iterator_group(stream: &mut TokenStream, group: &Group, separator: &TokenStream) -> Result<()> {
    let mut iter_idents = Vec::new();

    let output = parse_token_stream_in_iterator_pattern(group.stream(), &mut iter_idents)?;

    let mut idents = iter_idents.iter();
    let first = match idents.next() {
        Some(first) => first,
        None => return Err(Error::new(group.span(), "Expected at least one iterator inside pattern.")),
    };
    let first = quote!{ #first };
    let idents_in_tuple = idents.fold(first, |previous, next| quote!{ (#previous, #next) });

    let mut idents = iter_idents.iter();
    let first = match idents.next() {
        Some(first) => first,
        None => return Err(Error::new(group.span(), "Expected at least one iterator inside pattern.")),
    };
    let first_into_iter = quote_spanned!(first.span()=> #first .into_iter());
    let zip_iterators = idents.map(|ident| quote_spanned! { ident.span()=> .zip( #ident .into_iter() ) });
    if separator.is_empty() {
        stream.append_all(quote! {
            for #idents_in_tuple in #first_into_iter #(#zip_iterators)* {
                #output
            }
        });
    } else {
        stream.append_all(quote! {
            for (__i, #idents_in_tuple) in #first_into_iter #(#zip_iterators)* .enumerate() {
                if __i > 0 {
                    #separator
                }
                #output
            }
        });
    }

    Ok(())
}

/// Parses the input according to `quote!` rules.
fn parse_token_stream(input: TokenStream) -> Result<TokenStream> {
    let mut output = TokenStream::new();

    let mut input = input.into_iter().peekable();
    while let Some(token) = input.next() {
        match &token {
            TokenTree::Group(group) => parse_group(&mut output, group)?,
            TokenTree::Ident(ident) => parse_ident(&mut output, ident),
            TokenTree::Literal(lit) => parse_literal(&mut output, lit),
            TokenTree::Punct(punct) => {
                match interpolation_pattern_type(&punct, &mut input)? {
                    InterpolationPattern::Ident(ident) => {
                        interpolate_to_tokens_ident(&mut output, &ident)
                    },
                    InterpolationPattern::Iterator(group, separator) => {
                        interpolate_iterator_group(&mut output, &group, &separator)?
                    },
                    InterpolationPattern::None => {
                        parse_punct(&mut output, punct);
                    },
                }
            }
        }
    }

    Ok(output)
}

/// Parses the input according to `quote!` rules inside an iterator pattern.
fn parse_token_stream_in_iterator_pattern(
    input: TokenStream,
    iter_idents: &mut Vec<Ident>,
) -> Result<TokenStream> {
    let mut output = TokenStream::new();

    let mut input = input.into_iter().peekable();
    while let Some(token) = input.next() {
        match &token {
            TokenTree::Group(group) => {
                parse_group_in_iterator_pattern(&mut output, group, iter_idents)?
            }
            TokenTree::Ident(ident) => parse_ident(&mut output, ident),
            TokenTree::Literal(lit) => parse_literal(&mut output, lit),
            TokenTree::Punct(punct) => {
                match interpolation_pattern_type(&punct, &mut input)? {
                    InterpolationPattern::Ident(ident) => {
                        interpolate_to_tokens_ident(&mut output, &ident);
                        if !iter_idents.iter().any(|i| i == &ident) {
                            iter_idents.push(ident);
                        }
                    },
                    InterpolationPattern::Iterator(group, separator) => {
                        let span_s = group.span();
                        let span_e = separator.into_iter().last().map(|s| s.span()).unwrap_or(span_s);
                        return Err(Error::new(span_s, "Nested iterator patterns not supported.").end_span(span_e));
                    },
                    InterpolationPattern::None => {
                        parse_punct(&mut output, punct);
                    },
                }
            }
        }
    }

    Ok(output)
}

/// Parses the input according to `quote!` rules in an iterator pattern, between 
/// the parenthesis and the asterisk.
fn parse_separator(input: &mut InputIter, iterators_span: Span) -> Result<TokenStream> {
    let mut output = TokenStream::new();

    while let Some(token) = input.next() {
        match &token {
            TokenTree::Group(group) => parse_group(&mut output, group)?,
            TokenTree::Ident(ident) => parse_ident(&mut output, ident),
            TokenTree::Literal(lit) => parse_literal(&mut output, lit),
            TokenTree::Punct(punct) => {
                if punct.as_char() == '*' {
                    // The asterisk marks the end of the iterator pattern
                    return Ok(output);
                } else {
                    match interpolation_pattern_type(&punct, input)? {
                        InterpolationPattern::Ident(ident) => {
                            // TODO don't allow iterator variables
                            interpolate_to_tokens_ident(&mut output, &ident)
                        },
                        InterpolationPattern::Iterator(group, separator) => {
                            let span_s = group.span();
                            let span_e = separator.into_iter().last().map(|s| s.span()).unwrap_or(span_s);
                            return Err(Error::new(span_s, "Nested iterator patterns not supported.").end_span(span_e));
                        },
                        InterpolationPattern::None => {
                            parse_punct(&mut output, punct);
                        },
                    }
                }
            }
        }
    }

    Err(Error::new(iterators_span, "Iterating interpolation does not have `*` symbol."))
}

pub fn quote(input: TokenStream) -> TokenStream {
    match parse_token_stream(input) {
        Ok(output) => generate_quote_header(output),
        Err(err) => err.raise(),
    }
}
