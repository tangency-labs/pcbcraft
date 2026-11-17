# Object-query syntax

`pcbcraft_engine::query::parse` parses text into an owned syntax tree or a typed
error. It is a reusable engine API, not a selection command: no query evaluator,
GUI search, MCP tool, paging or area test is provided yet. Parsing requires no
project and does not read or modify documents, revision, selection or undo.

## Grammar

```text
query       = disjunction EOF
 disjunction = conjunction ( ("or" | "||") conjunction )*
 conjunction = unary ( ("and" | "&&") unary )*
 unary       = ("not" | "!") unary | "(" disjunction ")" | predicate
 predicate   = "true" | "false" | selector text_op string
             | "property" "(" string ")" number_op number
 selector    = "kind" | "layer" | "net" | "net_class"
             | "part_reference" | "part_group" | "footprint"
 text_op     = "=" | "!="
 number_op   = text_op | "<" | "<=" | ">" | ">="
 number      = [sign] digits ["." digits] [("e" | "E") [sign] digits]
 sign        = "+" | "-"
 digits      = ASCII_DIGIT+
 string      = JSON_DOUBLE_QUOTED_STRING
```

Whitespace between tokens is optional, except where needed to separate words or
numbers. Unicode whitespace is accepted outside strings. Keywords and selectors
are lowercase and case-sensitive. Text values require double quotes; JSON escapes,
including surrogate pairs, are decoded. Invalid escapes, lone surrogates and raw
control characters are refused. Empty text is a valid comparison value; a property
identifier must not be empty. String comparison is intended to be exact and
case-sensitive, with no wildcard, regular-expression or implicit substring syntax.

Comparisons form predicates; negation binds before conjunction, then disjunction.
The word and symbol forms have the same precedence. Parentheses override it.
This is PCBCraft syntax, not an implementation of another application's query
language or its operator precedence. For example:

```text
kind = "track" and net != "GND"
not (layer = "F.Cu" or layer = "B.Cu")
part_reference = "R1" or part_group = "Analog"
property("width_nm") >= 254000 and kind = "track"
```

`kind` addresses an object kind, `layer` a layer name, `net` a net name,
`net_class` a net-class name, `part_reference` a placed part instance's reference,
`part_group` a part-group name and `footprint` a footprint definition identifier.
The parser retains their strings without resolving them. A syntactically valid
query need not name an existing kind, layer, part group or property; that requires
a future evaluator with explicit project and sheet/board targets. Parsing does
not establish that an object kind or group exists in the current model.

Numeric properties keep a nonempty identifier and an exact canonical decimal
string. Numbers have an optional sign, at least one integer digit, an optional
nonempty fractional part and an optional signed decimal exponent. Leading zeros
are accepted and normalized. `+002.500` becomes `2.5`, `1e3` becomes `1000`,
and `-0.0` becomes `0`. The shared [quantity normalizer](quantity-units.md) bounds
input to 128 bytes, exponents to ±48, canonical numbers to 36 total digits and
18 fractional places, with no binary floating-point conversion or rounding.
The canonical leading zero counts as a digit. A numeric literal has no unit
suffix: its meaning is the property's canonical unit, which the evaluator must
resolve from typed metadata, never from a reference or display label. No unit or
property type is inferred during parsing. Numeric availability, unknown values
and dimensional compatibility remain evaluator work.

## Trees, errors and bounds

`Query.expression` contains `Expression { span, kind }`. `span` is a half-open
zero-based UTF-8 byte range into the original text. A grouped expression includes
its parentheses in its outer span; its children keep their own spans. Repeated
AND or OR operators at one level form a flat vector, not a deep binary chain.
Predicates retain typed selectors/comparisons, decoded strings and canonical
numeric text. These Rust types are not a serialized project or plugin format.

On failure, `ParseError` has a `kind`, `offset` and short `message`. Categories
are `UnexpectedToken`, `UnknownField`, `InvalidComparison`, `InvalidString`,
`InvalidNumber` and `LimitExceeded`. The offset is a zero-based **byte** position,
not a Unicode character index. It names the offending token; missing input uses
the input length. A malformed quoted string points to its opening quote, and an
invalid number to its first byte. An empty property identifier points to the
`property` selector. For `net = "電源" and )`, the unexpected `)` is byte 19.
Display text includes the byte position. Failure returns no partial tree.

Bounds apply before or during parsing:

| Resource | Inclusive limit | Refusal position |
| --- | --- | --- |
| Complete UTF-8 input, including whitespace | 16,384 bytes | 0 |
| Non-EOF lexical tokens | 4,096 | Start of token 4,097 |
| Combined open parentheses and unary negations | 32 | First excessive opening token |
| Numeric input and precision | Shared quantity bounds above | First byte of number |

Scanning, string decoding and numeric normalization process bounded input. The
lexer keeps only its current token. The tree is bounded by the token count;
flat chains avoid input-length recursion during parsing, cloning and dropping.
Nested boolean groups/negations use the depth bound. No cache, journal, project
snapshot or undo copy is retained by the parser.

## Extension and compatibility boundary

The syntax layer owns no object identity or revision. A later evaluator must
resolve against an explicit immutable project/board/sheet snapshot and carry
stable object IDs and the source revision in results, rather than use result
positions, display names or paths as permanent identities. A parsed reference
or name is a selector, not a new object ID. Rule-scope resolution, rectangle and
polygon predicates, deterministic paging and bulk commands remain separate work.
The non-exhaustive expression/predicate/selector enums leave room for those
predicates without adding a second parser or changing saved documents.

A plugin or cross-application adapter may reuse parsing as domain computation;
it still needs a versioned host protocol, discovery, target identity, pinned
revision, units and transforms. Parsing alone does not implement a plugin API,
live SolidCraft/Fieldcraft exchange or externally driven parametric updates.
Future outline updates must use reviewed revision-checked engine transactions;
no syntax node may mutate either application's session or native files directly.
No native format, migration, command catalog or lock entry changes here.

## Verification

`query::tests::forty_valid_queries_have_the_listed_trees` compares 40 original
queries with explicit tree forms. `twenty_invalid_queries_have_the_listed_diagnostics`
compares 20 refusals with category, byte offset and reason, including multibyte
text. Additional tests exercise each selector/comparison rule, decimal edges
(including integers beyond 2^53), byte/token/depth boundaries, malformed Unicode
escapes and original-source spans. A shared-command-created synthetic project
checks that parsing leaves its snapshot, encoded documents and undo unchanged.
These tests are syntax proof, not evaluator performance or UI acceptance.
