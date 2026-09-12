/**
 * Syntax highlighting, from scratch.
 *
 * Every highlighter worth having is a dependency of a megabyte or more, and
 * this app takes none. So this is the small honest version: a scanner per
 * family of language, producing a flat list of `{ text, kind }` spans that the
 * renderer paints. It is not a parser and does not pretend to be - it will
 * colour a keyword inside a regular expression wrong the way every regex-based
 * highlighter does - but it gets comments, strings, numbers, keywords, types
 * and call sites right, which is what makes code readable at a glance.
 *
 * Two rules keep it honest:
 *
 *  - Nothing is ever dropped. `tokenize(code, lang).map(t => t.text).join("")`
 *    is the input, character for character, for every language and every
 *    malformed fragment. A highlighter that eats a character is worse than no
 *    highlighter, because the reader copies what they see.
 *  - An unknown language is not an error. It comes back as one plain span, so
 *    a fence tagged `mermaid`, `prisma` or nothing at all renders as it always
 *    did.
 *
 * Kinds: comment, string, number, keyword, type, function, property,
 * operator, punctuation, tag, attribute, meta, added, removed, plain.
 */

const ALIASES = {
  javascript: "js",
  jsx: "js",
  mjs: "js",
  cjs: "js",
  node: "js",
  typescript: "ts",
  tsx: "ts",
  py: "python",
  python3: "python",
  rb: "ruby",
  sh: "shell",
  bash: "shell",
  zsh: "shell",
  console: "shell",
  shellsession: "shell",
  ps1: "powershell",
  pwsh: "powershell",
  yml: "yaml",
  htm: "html",
  xml: "html",
  svg: "html",
  vue: "html",
  scss: "css",
  less: "css",
  golang: "go",
  rs: "rust",
  kt: "kotlin",
  cs: "csharp",
  "c++": "cpp",
  cc: "cpp",
  h: "cpp",
  hpp: "cpp",
  objc: "cpp",
  patch: "diff",
  conf: "ini",
  toml: "ini",
  dockerfile: "shell",
  make: "shell",
  makefile: "shell",
};

const words = (list) => new Set(list.split(/\s+/).filter(Boolean));

const JS_KEYWORDS =
  "as async await break case catch class const continue debugger default delete do else export " +
  "extends finally for from function get if import in instanceof let new of return set static " +
  "super switch this throw try typeof var void while with yield";
const JS_LITERALS = "true false null undefined NaN Infinity";
const JS_TYPES =
  "Array Boolean Date Error Function JSON Map Math Number Object Promise RegExp Set String Symbol " +
  "WeakMap WeakSet BigInt Intl Proxy Reflect console document window globalThis process";

const TS_EXTRA =
  "abstract any asserts bigint boolean declare enum implements infer interface is keyof namespace " +
  "never number object override private protected public readonly require satisfies string symbol " +
  "type unique unknown";

const SPECS = {
  js: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"', "`"],
    keywords: words(JS_KEYWORDS),
    literals: words(JS_LITERALS),
    types: words(JS_TYPES),
  },
  ts: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"', "`"],
    keywords: words(`${JS_KEYWORDS} ${TS_EXTRA}`),
    literals: words(JS_LITERALS),
    types: words(JS_TYPES),
  },
  json: {
    line: [],
    block: [],
    quotes: ['"'],
    keywords: new Set(),
    literals: words("true false null"),
    types: new Set(),
  },
  python: {
    line: ["#"],
    block: [],
    quotes: ["'", '"'],
    triple: ['"""', "'''"],
    keywords: words(
      "and as assert async await break class continue def del elif else except finally for from " +
        "global if import in is lambda nonlocal not or pass raise return try while with yield match case"
    ),
    literals: words("True False None self cls"),
    types: words(
      "bool bytes dict float frozenset int list object set str tuple type Exception ValueError " +
        "TypeError KeyError RuntimeError Optional List Dict Any Union print len range enumerate zip open"
    ),
    decorator: true,
  },
  go: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"', "`"],
    keywords: words(
      "break case chan const continue default defer else fallthrough for func go goto if import " +
        "interface map package range return select struct switch type var"
    ),
    literals: words("true false nil iota"),
    types: words(
      "bool byte complex64 complex128 error float32 float64 int int8 int16 int32 int64 rune string " +
        "uint uint8 uint16 uint32 uint64 uintptr any make new len cap append copy delete panic recover"
    ),
  },
  rust: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"'],
    keywords: words(
      "as async await break const continue crate dyn else enum extern fn for if impl in let loop " +
        "match mod move mut pub ref return self Self static struct super trait type unsafe use where while"
    ),
    literals: words("true false None Some Ok Err"),
    types: words(
      "bool char f32 f64 i8 i16 i32 i64 i128 isize str u8 u16 u32 u64 u128 usize String Vec Option " +
        "Result Box Rc Arc HashMap HashSet"
    ),
  },
  java: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"'],
    keywords: words(
      "abstract assert break case catch class const continue default do else enum extends final " +
        "finally for goto if implements import instanceof interface native new package private " +
        "protected public return static strictfp super switch synchronized this throw throws " +
        "transient try var volatile while record sealed yield"
    ),
    literals: words("true false null"),
    types: words(
      "boolean byte char double float int long short void String Object Integer Double Boolean List " +
        "Map Set ArrayList HashMap Exception System"
    ),
    annotation: true,
  },
  csharp: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"'],
    keywords: words(
      "abstract as async await base break case catch checked class const continue default delegate " +
        "do else enum event explicit extern finally fixed for foreach goto if implicit in interface " +
        "internal is lock namespace new operator out override params private protected public " +
        "readonly ref return sealed sizeof stackalloc static struct switch this throw try typeof " +
        "unchecked unsafe using var virtual void volatile while record init with"
    ),
    literals: words("true false null value"),
    types: words(
      "bool byte char decimal double float int long object sbyte short string uint ulong ushort " +
        "Task List Dictionary IEnumerable Console String Exception"
    ),
    annotation: true,
  },
  cpp: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"'],
    keywords: words(
      "alignas alignof auto break case catch class const constexpr continue decltype default delete " +
        "do else enum explicit export extern for friend goto if inline mutable namespace new noexcept " +
        "operator private protected public register return sizeof static struct switch template this " +
        "throw try typedef typename union using virtual volatile while"
    ),
    literals: words("true false nullptr NULL"),
    types: words(
      "bool char double float int long short signed unsigned void size_t string vector map set " +
        "shared_ptr unique_ptr std uint8_t uint32_t int32_t int64_t"
    ),
    hash: true,
  },
  php: {
    line: ["//", "#"],
    block: [["/*", "*/"]],
    quotes: ["'", '"'],
    keywords: words(
      "abstract and array as break callable case catch class clone const continue declare default do " +
        "echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile enum extends " +
        "final finally fn for foreach function global goto if implements include include_once " +
        "instanceof insteadof interface isset list match namespace new or print private protected " +
        "public readonly require require_once return static switch throw trait try unset use var while xor yield"
    ),
    literals: words("true false null this"),
    types: words("bool int float string array object void mixed self parent"),
  },
  ruby: {
    line: ["#"],
    block: [],
    quotes: ["'", '"'],
    keywords: words(
      "alias and begin break case class def defined? do else elsif end ensure for if in module next " +
        "not or redo rescue retry return self super then undef unless until when while yield require " +
        "require_relative attr_accessor attr_reader attr_writer"
    ),
    literals: words("true false nil"),
    types: words("Array Hash String Symbol Integer Float Proc Struct Module Class Exception"),
  },
  sql: {
    line: ["--"],
    block: [["/*", "*/"]],
    quotes: ["'", '"', "`"],
    caseless: true,
    keywords: words(
      "add all alter and as asc between by case cast column constraint create cross database default " +
        "delete desc distinct drop else end exists foreign from full group having if in index inner " +
        "insert into is join key left like limit not null offset on or order outer primary references " +
        "right select set table then union unique update values view when where with returning"
    ),
    literals: words("true false null"),
    types: words(
      "bigint boolean bytea char date decimal double float int integer json jsonb numeric real " +
        "serial smallint text time timestamp uuid varchar"
    ),
  },
  shell: {
    line: ["#"],
    block: [],
    quotes: ["'", '"'],
    keywords: words(
      "if then elif else fi for while until do done case esac function in return break continue " +
        "local export set unset source alias declare readonly trap exit shift eval"
    ),
    literals: words("true false"),
    types: words(
      "cd echo ls cat grep sed awk find git npm npx node python pip docker kubectl make curl wget " +
        "mkdir rm cp mv chmod chown sudo apt brew ssh tar which printf read test"
    ),
    variable: true,
  },
  powershell: {
    line: ["#"],
    block: [["<#", "#>"]],
    quotes: ["'", '"'],
    caseless: true,
    keywords: words(
      "begin break catch class continue data define do dynamicparam else elseif end enum exit " +
        "filter finally for foreach from function hidden if in param process return switch throw try " +
        "until using var while"
    ),
    literals: words("$true $false $null"),
    types: words(
      "Get-ChildItem Get-Content Set-Content Write-Host Write-Output New-Item Remove-Item Copy-Item " +
        "Move-Item Select-Object Where-Object ForEach-Object Test-Path Join-Path Invoke-WebRequest"
    ),
    variable: true,
  },
  kotlin: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ["'", '"'],
    keywords: words(
      "as break by catch class companion const continue crossinline data do else enum external " +
        "false finally for fun get if import in infix init inline interface internal is lateinit " +
        "noinline null object open operator out override package private protected public reified " +
        "return sealed set super suspend this throw try typealias val var vararg when while"
    ),
    literals: words("true false null it"),
    types: words("Any Boolean Byte Char Double Float Int List Long Map Set String Unit Array Nothing"),
    annotation: true,
  },
  swift: {
    line: ["//"],
    block: [["/*", "*/"]],
    quotes: ['"'],
    keywords: words(
      "associatedtype break case catch class continue default defer deinit do else enum extension " +
        "fallthrough fileprivate final for func guard if import in init inout internal let open " +
        "operator private protocol public repeat rethrows return self static struct subscript super " +
        "switch throw throws try typealias var where while async await"
    ),
    literals: words("true false nil"),
    types: words("Any Array Bool Character Dictionary Double Float Int Optional Set String Void"),
    hash: true,
  },
};

/** The language a fence tag means, or null when nothing here knows it. */
export function languageOf(tag) {
  const name = String(tag ?? "")
    .trim()
    .toLowerCase();
  if (!name) return null;
  const resolved = ALIASES[name] ?? name;
  if (SPECS[resolved]) return resolved;
  if (["html", "css", "diff", "yaml", "ini", "markdown", "md"].includes(resolved)) return resolved;
  return null;
}

/* -- the scanners --------------------------------------------------------- */

/**
 * Spans, accumulated. Adjacent spans of the same kind are merged, which halves
 * the element count for ordinary code - a line of prose in a comment would
 * otherwise be one span per word.
 */
function collector() {
  const out = [];
  return {
    out,
    push(kind, text) {
      if (!text) return;
      const last = out[out.length - 1];
      if (last && last.kind === kind) last.text += text;
      else out.push({ kind, text });
    },
  };
}

const IDENT_START = /[A-Za-z_$]/;
const IDENT_PART = /[A-Za-z0-9_$]/;
const DIGIT = /[0-9]/;
const OPERATOR = /[+\-*/%=<>!&|^~?:]/;
const PUNCTUATION = /[{}()[\];,.]/;

function readString(code, start, quote, allowNewline) {
  let i = start + quote.length;
  while (i < code.length) {
    const ch = code[i];
    if (ch === "\\") {
      i += 2;
      continue;
    }
    if (!allowNewline && ch === "\n") return i;
    if (code.startsWith(quote, i)) return i + quote.length;
    i += 1;
  }
  return code.length;
}

function readNumber(code, start) {
  let i = start;
  if (code[i] === "0" && /[xXbBoO]/.test(code[i + 1] ?? "")) {
    i += 2;
    while (i < code.length && /[0-9a-fA-F_]/.test(code[i])) i += 1;
    return i;
  }
  while (i < code.length && /[0-9_]/.test(code[i])) i += 1;
  if (code[i] === "." && DIGIT.test(code[i + 1] ?? "")) {
    i += 1;
    while (i < code.length && /[0-9_]/.test(code[i])) i += 1;
  }
  if (/[eE]/.test(code[i] ?? "") && /[0-9+-]/.test(code[i + 1] ?? "")) {
    i += 2;
    while (i < code.length && DIGIT.test(code[i])) i += 1;
  }
  while (i < code.length && /[a-zA-Z]/.test(code[i])) i += 1; // 10n, 3.5f, 1u32
  return i;
}

function scanCurly(code, spec) {
  const { out, push } = collector();
  const len = code.length;
  let i = 0;
  let previous = "";

  const wordKind = (word, next) => {
    const key = spec.caseless ? word.toLowerCase() : word;
    if (spec.keywords.has(key)) return "keyword";
    if (spec.literals.has(key)) return "literal-keyword";
    if (spec.types.has(key)) return "type";
    if (next === "(") return "function";
    if (previous === ".") return "property";
    // A capitalised bare word in a language with types is almost always one.
    if (spec.types.size && /^[A-Z][A-Za-z0-9_]*$/.test(word)) return "type";
    return "plain";
  };

  while (i < len) {
    const ch = code[i];

    if (ch === "\n" || ch === " " || ch === "\t" || ch === "\r") {
      push("plain", ch);
      i += 1;
      continue;
    }

    const line = spec.line.find((marker) => code.startsWith(marker, i));
    if (line) {
      const end = code.indexOf("\n", i);
      push("comment", code.slice(i, end === -1 ? len : end));
      i = end === -1 ? len : end;
      continue;
    }

    const block = (spec.block ?? []).find(([open]) => code.startsWith(open, i));
    if (block) {
      const end = code.indexOf(block[1], i + block[0].length);
      const stop = end === -1 ? len : end + block[1].length;
      push("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    const triple = (spec.triple ?? []).find((quote) => code.startsWith(quote, i));
    if (triple) {
      const end = readString(code, i, triple, true);
      push("string", code.slice(i, end));
      i = end;
      previous = '"';
      continue;
    }

    if (spec.quotes.includes(ch)) {
      // A backtick string spans lines; a plain one does not, so an apostrophe
      // in a comment-free line of prose cannot swallow the rest of the file.
      const end = readString(code, i, ch, ch === "`");
      push("string", code.slice(i, end));
      i = end;
      previous = ch;
      continue;
    }

    if (spec.variable && ch === "$") {
      let j = i + 1;
      if (code[j] === "{") {
        j = code.indexOf("}", j);
        j = j === -1 ? len : j + 1;
      } else while (j < len && IDENT_PART.test(code[j])) j += 1;
      push("property", code.slice(i, j));
      i = j;
      previous = "$";
      continue;
    }

    if (spec.decorator && ch === "@" && IDENT_START.test(code[i + 1] ?? "")) {
      let j = i + 1;
      while (j < len && (IDENT_PART.test(code[j]) || code[j] === ".")) j += 1;
      push("meta", code.slice(i, j));
      i = j;
      continue;
    }

    if (spec.annotation && ch === "@" && IDENT_START.test(code[i + 1] ?? "")) {
      let j = i + 1;
      while (j < len && IDENT_PART.test(code[j])) j += 1;
      push("meta", code.slice(i, j));
      i = j;
      continue;
    }

    if (spec.hash && ch === "#" && IDENT_START.test(code[i + 1] ?? "")) {
      const end = code.indexOf("\n", i);
      push("meta", code.slice(i, end === -1 ? len : end));
      i = end === -1 ? len : end;
      continue;
    }

    if (DIGIT.test(ch)) {
      const end = readNumber(code, i);
      push("number", code.slice(i, end));
      i = end;
      previous = "0";
      continue;
    }

    if (IDENT_START.test(ch)) {
      let j = i;
      while (j < len && IDENT_PART.test(code[j])) j += 1;
      const word = code.slice(i, j);
      let after = j;
      while (after < len && (code[after] === " " || code[after] === "\t")) after += 1;
      const kind = wordKind(word, code[after]);
      push(kind === "literal-keyword" ? "keyword" : kind, word);
      i = j;
      previous = word;
      continue;
    }

    if (OPERATOR.test(ch)) {
      let j = i;
      while (j < len && OPERATOR.test(code[j])) j += 1;
      push("operator", code.slice(i, j));
      i = j;
      previous = code[j - 1];
      continue;
    }

    push(PUNCTUATION.test(ch) ? "punctuation" : "plain", ch);
    previous = ch;
    i += 1;
  }

  return out;
}

/** JSON, where a string before a colon is a key rather than a value. */
function scanJson(code) {
  const spans = scanCurly(code, SPECS.json);
  for (let i = 0; i < spans.length; i += 1) {
    if (spans[i].kind !== "string") continue;
    let j = i + 1;
    while (j < spans.length && !spans[j].text.trim()) j += 1;
    if (spans[j]?.text.startsWith(":")) spans[i] = { ...spans[i], kind: "property" };
  }
  return spans;
}

function scanHtml(code) {
  const { out, push } = collector();
  const len = code.length;
  let i = 0;

  while (i < len) {
    if (code.startsWith("<!--", i)) {
      const end = code.indexOf("-->", i);
      const stop = end === -1 ? len : end + 3;
      push("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    if (code[i] !== "<") {
      const next = code.indexOf("<", i);
      push("plain", code.slice(i, next === -1 ? len : next));
      i = next === -1 ? len : next;
      continue;
    }

    // Inside a tag: the name is a tag, the words are attributes, the quoted
    // runs are strings, and everything else is punctuation.
    let j = i + 1;
    push("punctuation", "<");
    if (code[j] === "/" || code[j] === "!" || code[j] === "?") {
      push("punctuation", code[j]);
      j += 1;
    }
    let k = j;
    while (k < len && /[A-Za-z0-9_:.-]/.test(code[k])) k += 1;
    push("tag", code.slice(j, k));
    j = k;

    while (j < len && code[j] !== ">") {
      const ch = code[j];
      if (ch === '"' || ch === "'") {
        const end = readString(code, j, ch, true);
        push("string", code.slice(j, end));
        j = end;
        continue;
      }
      if (/[A-Za-z_@:[(]/.test(ch)) {
        let m = j;
        while (m < len && /[A-Za-z0-9_@:[\]().-]/.test(code[m])) m += 1;
        push("attribute", code.slice(j, m));
        j = m;
        continue;
      }
      push(ch === "=" ? "operator" : "plain", ch);
      j += 1;
    }
    if (j < len) {
      push("punctuation", ">");
      j += 1;
    }
    i = j;
  }

  return out;
}

function scanCss(code) {
  const { out, push } = collector();
  const len = code.length;
  let i = 0;
  let inBlock = false;
  let expectingValue = false;

  while (i < len) {
    const ch = code[i];

    if (code.startsWith("/*", i)) {
      const end = code.indexOf("*/", i);
      const stop = end === -1 ? len : end + 2;
      push("comment", code.slice(i, stop));
      i = stop;
      continue;
    }

    if (ch === '"' || ch === "'") {
      const end = readString(code, i, ch, false);
      push("string", code.slice(i, end));
      i = end;
      continue;
    }

    if (ch === "{") {
      inBlock = true;
      expectingValue = false;
      push("punctuation", ch);
      i += 1;
      continue;
    }
    if (ch === "}") {
      inBlock = false;
      expectingValue = false;
      push("punctuation", ch);
      i += 1;
      continue;
    }
    if (ch === ":" && inBlock) {
      expectingValue = true;
      push("punctuation", ch);
      i += 1;
      continue;
    }
    if (ch === ";") {
      expectingValue = false;
      push("punctuation", ch);
      i += 1;
      continue;
    }

    if (ch === "@") {
      let j = i + 1;
      while (j < len && /[A-Za-z-]/.test(code[j])) j += 1;
      push("keyword", code.slice(i, j));
      i = j;
      continue;
    }

    if (ch === "#" && /[0-9a-fA-F]/.test(code[i + 1] ?? "")) {
      let j = i + 1;
      while (j < len && /[0-9a-fA-F]/.test(code[j])) j += 1;
      push("number", code.slice(i, j));
      i = j;
      continue;
    }

    if (DIGIT.test(ch)) {
      let j = readNumber(code, i);
      while (j < len && /[a-z%]/.test(code[j])) j += 1;
      push("number", code.slice(i, j));
      i = j;
      continue;
    }

    if (/[A-Za-z_.#*[-]/.test(ch)) {
      let j = i;
      while (j < len && /[A-Za-z0-9_.#*[\]="'|~^$-]/.test(code[j])) j += 1;
      const word = code.slice(i, j);
      let after = j;
      while (after < len && /\s/.test(code[after])) after += 1;
      const kind = !inBlock
        ? "tag"
        : expectingValue
          ? code[after] === "("
            ? "function"
            : "plain"
          : "property";
      push(kind, word);
      i = j;
      continue;
    }

    push(/\s/.test(ch) ? "plain" : "punctuation", ch);
    i += 1;
  }

  return out;
}

/** A line-oriented scanner, for the formats where the first character decides. */
function scanLines(code, kindOf) {
  const { out, push } = collector();
  const lines = code.split("\n");
  lines.forEach((line, index) => {
    kindOf(line, push);
    if (index < lines.length - 1) push("plain", "\n");
  });
  return out;
}

function scanDiff(code) {
  return scanLines(code, (line, push) => {
    if (/^(\+\+\+|---|diff |index |@@)/.test(line)) push("meta", line);
    else if (line.startsWith("+")) push("added", line);
    else if (line.startsWith("-")) push("removed", line);
    else if (line.startsWith("\\")) push("comment", line);
    else push("plain", line);
  });
}

function scanYaml(code) {
  return scanLines(code, (line, push) => {
    const comment = line.indexOf("#");
    const body = comment === -1 ? line : line.slice(0, comment);
    const key = /^(\s*(?:-\s+)?)([A-Za-z0-9_.$/-]+)(\s*:)(.*)$/.exec(body);
    if (key) {
      push("plain", key[1]);
      push("property", key[2]);
      push("punctuation", key[3]);
      const value = key[4];
      if (/^\s*(true|false|null|~)\s*$/i.test(value)) push("keyword", value);
      else if (/^\s*-?\d[\d._]*\s*$/.test(value)) push("number", value);
      else if (/^\s*["']/.test(value)) push("string", value);
      else push("plain", value);
    } else if (/^\s*-{3}\s*$/.test(body)) {
      push("meta", body);
    } else {
      push("plain", body);
    }
    if (comment !== -1) push("comment", line.slice(comment));
  });
}

function scanIni(code) {
  return scanLines(code, (line, push) => {
    if (/^\s*[#;]/.test(line)) {
      push("comment", line);
      return;
    }
    if (/^\s*\[/.test(line)) {
      push("tag", line);
      return;
    }
    const pair = /^(\s*)([A-Za-z0-9_.$-]+)(\s*=\s*)(.*)$/.exec(line);
    if (!pair) {
      push("plain", line);
      return;
    }
    push("plain", pair[1]);
    push("property", pair[2]);
    push("operator", pair[3]);
    if (/^-?\d/.test(pair[4])) push("number", pair[4]);
    else if (/^(true|false)$/i.test(pair[4].trim())) push("keyword", pair[4]);
    else push("string", pair[4]);
  });
}

function scanMarkdown(code) {
  return scanLines(code, (line, push) => {
    if (/^\s*#{1,6}\s/.test(line)) push("keyword", line);
    else if (/^\s*(```|~~~)/.test(line)) push("meta", line);
    else if (/^\s*>/.test(line)) push("comment", line);
    else if (/^\s*([-*+]|\d+[.)])\s/.test(line)) {
      const marker = /^\s*([-*+]|\d+[.)])\s/.exec(line)[0];
      push("punctuation", marker);
      push("plain", line.slice(marker.length));
    } else push("plain", line);
  });
}

const SCANNERS = {
  json: scanJson,
  html: scanHtml,
  css: scanCss,
  diff: scanDiff,
  yaml: scanYaml,
  ini: scanIni,
  markdown: scanMarkdown,
  md: scanMarkdown,
};

/**
 * Spans for this code, in this language.
 *
 * Always returns something: an unknown language, an empty string or a language
 * this cannot parse all come back as a single plain span, so a caller never
 * has to branch on whether highlighting happened.
 */
export function tokenize(code, lang) {
  const text = String(code ?? "");
  if (!text) return [];
  const language = languageOf(lang);
  if (!language) return [{ kind: "plain", text }];
  try {
    const scanner = SCANNERS[language];
    const spans = scanner ? scanner(text) : scanCurly(text, SPECS[language]);
    return spans.length ? spans : [{ kind: "plain", text }];
  } catch {
    // A highlighter is decoration. It is never allowed to be the reason a
    // message fails to render.
    return [{ kind: "plain", text }];
  }
}
