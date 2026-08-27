<h1 align="center">Picomatch</h1>

<p align="center">
<strong>Blazing fast and accurate glob matcher, ported to Rust.</strong></br>
<em>No dependencies and full support for standard and extended Bash glob features, including braces, extglobs, POSIX brackets, and regular expressions.</em>
</p>

## About this port

This crate is a Rust port of [picomatch](https://github.com/micromatch/picomatch). Globs compile to the same regular expression source as in the JavaScript original, and the crate carries its own implementation of JavaScript regular expressions (`src/regexp`), so a pattern means exactly what it means in Node.js: matching counts UTF-16 code units, `nocase` follows JavaScript's case folding, invalid regular expressions never match, and regular expression features passed through parentheses and brackets behave as in JavaScript. The `u` and `v` flags of the `flags` option are not supported.

Build and test with cargo:

```sh
cargo build --release
cargo test
```

### The picomatch command

The `picomatch` binary takes no arguments. It reads one JSON request per line on standard input and writes one JSON response per line on standard output; the exit status is 1 when any response is an error and 0 otherwise.

```sh
$ echo '{"op":"isMatch","patterns":"*.js","inputs":["a.js","a.ts"]}' | picomatch
{"results":[true,false]}
```

| Request | Response |
| --- | --- |
| `{"op":"isMatch","patterns":P,"inputs":[...]}` | `{"results":[true,...]}` |
| `{"op":"match","patterns":P,"inputs":[...]}` | `{"results":[{"isMatch":...,"output":...,"match":...},...]}` |
| `{"op":"test","glob":G,"inputs":[...]}` | `{"results":[{"isMatch":...,"output":...,"match":...},...]}` |
| `{"op":"matchBase","glob":G,"input":S}` | `{"result":true}` |
| `{"op":"scan","input":S}` | the scan state |
| `{"op":"parse","pattern":P}` | the parse state without regular expression source |
| `{"op":"toRegex","source":S,"inputs":[...]}` | `{"results":[...]}` |

Every request can carry `options`, the picomatch options below, with `format` (`"slashes"` or `"lower"`) and `expandRange` (`"group"` or `"alternate"`) chosen by name. With `events`, `isMatch` and `match` also report the `onResult`, `onIgnore` and `onMatch` calls. A failed request gets `{"error":{"name":...,"message":...}}`.

### The library

```rust
use picomatch::js::{JsString, Value};
use picomatch::picomatch::Matcher;

let pattern = Value::str("*.js");
let mut matcher = Matcher::new(&pattern, None).expect("a valid pattern");
let result = matcher.call(&Value::String(JsString::from("a.js")), &mut Vec::new());
assert!(result.expect("a string input").is_match());
```

## Options

### Picomatch options

The following options may be used with the main `picomatch()` function or any of the methods on the picomatch API.

| **Option** | **Type** | **Default value** | **Description** |
| --- | --- | --- | --- |
| `basename`            | `boolean`      | `false`     | If set, then patterns without slashes will be matched against the basename of the path if it contains slashes.  For example, `a?b` would match the path `/xyz/123/acb`, but not `/xyz/acb/123`. |
| `bash`                | `boolean`      | `false`     | Follow bash matching rules more strictly - disallows backslashes as escape characters, and treats single stars as globstars (`**`). |
| `capture`             | `boolean`      | `undefined` | Return regex matches in supporting methods. |
| `contains`            | `boolean`      | `undefined` | Allows glob to match any part of the given string(s). |
| `debug`               | `boolean`      | `undefined` | Debug regular expressions when an error is thrown. |
| `dot`                 | `boolean`      | `false`     | Enable dotfile matching. By default, dotfiles are ignored unless a `.` is explicitly defined in the pattern, or `options.dot` is true |
| `expandRange`         | `function`     | `undefined` | Custom function for expanding ranges in brace patterns, such as `{a..z}`. The function receives the range values as two arguments, and it must return a string to be used in the generated regex. It's recommended that returned strings be wrapped in parentheses. |
| `fastpaths`           | `boolean`      | `true`      | To speed up processing, full parsing is skipped for a handful common glob patterns. Disable this behavior by setting this option to `false`. |
| `flags`               | `string`      | `undefined` | Regex flags to use in the generated regex. If defined, the `nocase` option will be overridden. |
| [format](#optionsformat) | `function` | `undefined` | Custom function for formatting the returned string. This is useful for removing leading slashes, converting Windows paths to Posix paths, etc. |
| `ignore`              | `array\|string` | `undefined` | One or more glob patterns for excluding strings that should not be matched from the result. |
| `keepQuotes`          | `boolean`      | `false`     | Retain quotes in the generated regex, since quotes may also be used as an alternative to backslashes.  |
| `literalBrackets`     | `boolean`      | `undefined` | When `true`, brackets in the glob pattern will be escaped so that only literal brackets will be matched. |
| `matchBase`           | `boolean`      | `false`     | Alias for `basename` |
| `maxLength`           | `number`      | `65536`     | Limit the max length of the input string. An error is thrown if the input string is longer than this value. |
| `maxExtglobRecursion` | `number\|boolean` | `0` | Limit nested quantified extglobs and other risky repeated extglob forms. When the limit is exceeded, the extglob is treated as a literal string instead of being compiled to regex. Set to `false` to disable this safeguard. |
| `nobrace`             | `boolean`      | `false`     | Disable brace matching, so that `{a,b}` and `{1..3}` would be treated as literal characters. |
| `nobracket`           | `boolean`      | `undefined` | Disable matching with regex brackets. |
| `nocase`              | `boolean`      | `false`     | Make matching case-insensitive. Equivalent to the regex `i` flag. Note that this option is overridden by the `flags` option. |
| `noext`               | `boolean`      | `false`     | Alias for `noextglob` |
| `noextglob`           | `boolean`      | `false`     | Disable support for matching with extglobs (like `+(a\|b)`) |
| `noglobstar`          | `boolean`      | `false`     | Disable support for matching nested directories with globstars (`**`) |
| `nonegate`            | `boolean`      | `false`     | Disable support for negating with leading `!` |
| [onIgnore](#optionsonIgnore) | `function` | `undefined` | Function to be called on ignored items. |
| [onMatch](#optionsonMatch) | `function` | `undefined` | Function to be called on matched items. |
| [onResult](#optionsonResult) | `function` | `undefined` | Function to be called on all items, regardless of whether or not they are matched or ignored. |
| `posix`               | `boolean`      | `false`     | Support POSIX character classes ("posix brackets"). |
| `prepend`             | `boolean`      | `undefined` | String to prepend to the generated regex used for matching. |
| `regex`               | `boolean`      | `false`     | Use regular expression rules for `+` (instead of matching literal `+`), and for stars that follow closing parentheses or brackets (as in `)*` and `]*`). |
| `strictBrackets`      | `boolean`      | `undefined` | Throw an error if brackets, braces, or parens are imbalanced. |
| `strictSlashes`       | `boolean`      | `undefined` | When true, picomatch won't match trailing slashes with single stars. |
| `unescape`            | `boolean`      | `undefined` | Remove backslashes preceding escaped characters in the glob pattern. By default, backslashes are retained. |
| `windows`             | `boolean`      | `false`     | Also accept backslashes as the path separator. |

### Scan Options

In addition to the main [picomatch options](#picomatch-options), the following options may also be used with the [.scan](#scan) method.

| **Option** | **Type** | **Default value** | **Description** |
| --- | --- | --- | --- |
| `tokens` | `boolean` | `false` | When `true`, the returned object will include an array of tokens (objects), representing each path "segment" in the scanned glob pattern |
| `parts` | `boolean` | `false` | When `true`, the returned object will include an array of strings representing each path "segment" in the scanned glob pattern. This is automatically enabled when `options.tokens` is true |

**Example**



### Options Examples

#### options.expandRange

**Type**: `function`

**Default**: `undefined`

Custom function for expanding ranges in brace patterns. The [fill-range](https://github.com/jonschlinkert/fill-range) library is ideal for this purpose, or you can use custom code to do whatever you need.

**Example**

The following example shows how to create a glob that matches a folder


#### options.format

**Type**: `function`

**Default**: `undefined`

Custom function for formatting strings before they're matched.

**Example**


## Globbing features

* [Basic globbing](#basic-globbing) (Wildcard matching)
* [Advanced globbing](#advanced-globbing) (extglobs, posix brackets, brace matching)

### Basic globbing

| **Character** | **Description** |
| --- | --- |
| `*` | Matches any character zero or more times, excluding path separators. Does _not match_ path separators or hidden files or directories ("dotfiles"), unless explicitly enabled by setting the `dot` option to `true`. |
| `**` | Matches any character zero or more times, including path separators. Note that `**` will only match path separators (`/`, and `\\` with the `windows` option) when they are the only characters in a path segment. Thus, `foo**/bar` is equivalent to `foo*/bar`, and `foo/a**b/bar` is equivalent to `foo/a*b/bar`, and _more than two_ consecutive stars in a glob path segment are regarded as _a single star_. Thus, `foo/***/bar` is equivalent to `foo/*/bar`. |
| `?` | Matches any character excluding path separators one time. Does _not match_ path separators or leading dots.  |
| `[abc]` | Matches any characters inside the brackets. For example, `[abc]` would match the characters `a`, `b` or `c`, and nothing else. |

#### Matching behavior vs. Bash

Picomatch's matching features and expected results in unit tests are based on Bash's unit tests and the Bash 4.3 specification, with the following exceptions:

* Bash will match `foo/bar/baz` with `*`. Picomatch only matches nested directories with `**`.
* Bash greedily matches with negated extglobs. For example, Bash 4.3 says that `!(foo)*` should match `foo` and `foobar`, since the trailing `*` bracktracks to match the preceding pattern. This is very memory-inefficient, and IMHO, also incorrect. Picomatch would return `false` for both `foo` and `foobar`.


### Advanced globbing

* [extglobs](#extglobs)
* [POSIX brackets](#posix-brackets)
* [Braces](#brace-expansion)

#### Extglobs

| **Pattern** | **Description** |
| --- | --- |
| `@(pattern)` | Match _only one_ consecutive occurrence of `pattern` |
| `*(pattern)` | Match _zero or more_ consecutive occurrences of `pattern` |
| `+(pattern)` | Match _one or more_ consecutive occurrences of `pattern` |
| `?(pattern)` | Match _zero or **one**_ consecutive occurrences of `pattern` |
| `!(pattern)` | Match _anything but_ `pattern` |

**Examples**


#### POSIX brackets

POSIX classes are disabled by default. Enable this feature by setting the `posix` option to true.

**Enable POSIX bracket support**


**Supported POSIX classes**

The following named POSIX bracket expressions are supported:

* `[:alnum:]` - Alphanumeric characters, equ `[a-zA-Z0-9]`
* `[:alpha:]` - Alphabetical characters, equivalent to `[a-zA-Z]`.
* `[:ascii:]` - ASCII characters, equivalent to `[\\x00-\\x7F]`.
* `[:blank:]` - Space and tab characters, equivalent to `[ \\t]`.
* `[:cntrl:]` - Control characters, equivalent to `[\\x00-\\x1F\\x7F]`.
* `[:digit:]` - Numerical digits, equivalent to `[0-9]`.
* `[:graph:]` - Graph characters, equivalent to `[\\x21-\\x7E]`.
* `[:lower:]` - Lowercase letters, equivalent to `[a-z]`.
* `[:print:]` - Print characters, equivalent to `[\\x20-\\x7E ]`.
* `[:punct:]` - Punctuation and symbols, equivalent to `[\\-!"#$%&\'()\\*+,./:;<=>?@[\\\\\\]^_`{|}~]`.
* `[:space:]` - Extended space characters, equivalent to `[ \\t\\r\\n\\v\\f]`.
* `[:upper:]` - Uppercase letters, equivalent to `[A-Z]`.
* `[:word:]` -  Word characters (letters, numbers and underscores), equivalent to `[A-Za-z0-9_]`.
* `[:xdigit:]` - Hexadecimal digits, equivalent to `[A-Fa-f0-9]`.

See the [Bash Reference Manual](https://www.gnu.org/software/bash/manual/html_node/Pattern-Matching.html) for more information.

### Braces

Picomatch only does [brace expansion](https://www.gnu.org/software/bash/manual/html_node/Brace-Expansion.html) of comma-delimited lists (e.g. `a/{b,c}/d`). For advanced matching with braces, use [micromatch](https://github.com/micromatch/micromatch), which supports advanced syntax such as ranges (e.g. `{01..03}`) and increments (e.g. `{2..10..2}`).

### Matching special characters as literals

If you wish to match the following special characters in a filepath, and you want to use these characters in your glob pattern, they must be escaped with backslashes or quotes:

**Special Characters**

Some characters that are used for matching in regular expressions are also regarded as valid file path characters on some platforms.

To match any of the following characters as literals: `$^*+?()[]

Examples:



## Philosophies

The goal of this library is to be blazing fast, without compromising on accuracy.

**Accuracy**

The number one of goal of this library is accuracy. However, it's not unusual for different glob implementations to have different rules for matching behavior, even with simple wildcard matching. It gets increasingly more complicated when combinations of different features are combined, like when extglobs are combined with globstars, braces, slashes, and so on: `!(**/{a,b,*/c})`.

Thus, given that there is no canonical glob specification to use as a single source of truth when differences of opinion arise regarding behavior, sometimes we have to implement our best judgement and rely on feedback from users to make improvements.

**Performance**

Although this library performs well in benchmarks, and in most cases it's faster than other popular libraries we benchmarked against, we will always choose accuracy over performance. It's not helpful to anyone if our library is faster at returning the wrong answer.

## License

Copyright © 2017-present, [Jon Schlinkert](https://github.com/jonschlinkert). Released under the [MIT License](LICENSE).
