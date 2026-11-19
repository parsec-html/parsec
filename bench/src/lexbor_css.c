/*
 * SPDX-License-Identifier: Apache-2.0
 * Copyright 2026 James Conway
 *
 * Lexbor's CSS tokenizer and stylesheet parser, called the way its own
 * examples call them (examples/lexbor/css/syntax/tokenizer/from_file.c and
 * examples/lexbor/css/StyleSheet.c), behind three functions. Reading a
 * token's type needs Lexbor's struct layout, which C has and Rust's
 * hand-declared FFI doesn't.
 */

#include <lexbor/css/css.h>

/* A tokenizer and a parser, made once and reused for every stylesheet. */
typedef struct {
    lxb_css_syntax_tokenizer_t *tkz;
    lxb_css_parser_t *parser;
} bench_css_t;

bench_css_t *bench_css_create(void)
{
    bench_css_t *b = lexbor_calloc(1, sizeof(bench_css_t));
    b->tkz = lxb_css_syntax_tokenizer_create();
    if (lxb_css_syntax_tokenizer_init(b->tkz) != LXB_STATUS_OK) {
        return NULL;
    }
    b->parser = lxb_css_parser_create();
    if (lxb_css_parser_init(b->parser, NULL) != LXB_STATUS_OK
        || lxb_css_parser_selectors_init(b->parser) != LXB_STATUS_OK)
    {
        return NULL;
    }
    return b;
}

void bench_css_destroy(bench_css_t *b)
{
    lxb_css_syntax_tokenizer_destroy(b->tkz);
    lxb_css_parser_destroy(b->parser, true);
    lexbor_free(b);
}

/* Tokenizes the whole of css, returning the number of tokens. */
size_t bench_css_tokens(bench_css_t *b, const lxb_char_t *css, size_t len)
{
    size_t n = 0;
    lxb_css_syntax_token_t *token;

    lxb_css_syntax_tokenizer_clean(b->tkz);
    lxb_css_syntax_tokenizer_buffer_set(b->tkz, css, len);
    for (;;) {
        token = lxb_css_syntax_token(b->tkz);
        if (token == NULL || token->type == LXB_CSS_SYNTAX_TOKEN__EOF) {
            break;
        }
        n++;
        lxb_css_syntax_token_consume(b->tkz);
    }
    return token == NULL ? 0 : n;
}

/* Parses css into a stylesheet, returning it to be freed later. */
lxb_css_stylesheet_t *bench_css_parse(bench_css_t *b, const lxb_char_t *css, size_t len)
{
    lxb_css_stylesheet_t *sst = lxb_css_stylesheet_create(NULL);
    lxb_css_parser_clean(b->parser);
    if (lxb_css_stylesheet_parse(sst, b->parser, css, len) != LXB_STATUS_OK) {
        lxb_css_stylesheet_destroy(sst, true);
        return NULL;
    }
    return sst;
}

void bench_css_free(lxb_css_stylesheet_t *sst)
{
    lxb_css_stylesheet_destroy(sst, true);
}
