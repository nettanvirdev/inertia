import * as React from "react";
import { cn } from "@/lib/utils";
import { parseMath } from "./parse.js";

/**
 * A formula, laid out.
 *
 * The tree from `parse.js` becomes nested spans, and CSS does the geometry:
 * a fraction is a flex column with a rule between its halves, a script is a
 * baseline shift, a root is a glyph beside an overlined body. That is how
 * every browser-side maths renderer works; this is the small version, and it
 * is enough for the formulas that turn up in an explanation.
 *
 * Display and inline differ in one thing that matters: in display mode the
 * limits of a sum or an integral sit above and below the sign, and inline they
 * sit beside it, because a message line cannot grow to three times its height
 * for one symbol.
 */

function Row({ node, display }) {
  return node.body.map((child, index) => <Node key={index} node={child} display={display} />);
}

function Scripts({ node, display }) {
  const stacked = node.limits && display && (node.sup || node.sub);
  if (stacked) {
    return (
      <span className="inline-flex flex-col items-center align-middle">
        {node.sup ? (
          <span className="text-[0.7em] leading-tight">
            <Node node={node.sup} display={false} />
          </span>
        ) : null}
        <span className="text-[1.35em] leading-none">
          <Node node={node.base} display={display} />
        </span>
        {node.sub ? (
          <span className="text-[0.7em] leading-tight">
            <Node node={node.sub} display={false} />
          </span>
        ) : null}
      </span>
    );
  }
  return (
    <span className="inline-block">
      <span className={cn(node.limits && "text-[1.25em] leading-none")}>
        <Node node={node.base} display={display} />
      </span>
      {node.sub ? (
        <sub className="text-[0.72em]">
          <Node node={node.sub} display={false} />
        </sub>
      ) : null}
      {node.sup ? (
        <sup className="text-[0.72em]">
          <Node node={node.sup} display={false} />
        </sup>
      ) : null}
    </span>
  );
}

function Fraction({ node }) {
  return (
    <span className="inline-flex flex-col items-center px-[0.15em] align-middle leading-tight">
      <span className="px-[0.25em] pb-[0.1em] text-[0.95em]">
        <Node node={node.num} display={false} />
      </span>
      {/* The rule is a border rather than a character: a glyph would not be
          the width of the wider of the two halves. */}
      <span className="w-full border-t border-current px-[0.25em] pt-[0.1em] text-[0.95em]">
        <Node node={node.den} display={false} />
      </span>
    </span>
  );
}

function Root({ node, display }) {
  return (
    <span className="inline-flex items-start align-middle">
      {node.index ? (
        <span className="mr-[-0.35em] translate-y-[-0.25em] text-[0.6em]">
          <Node node={node.index} display={false} />
        </span>
      ) : null}
      <span className="text-[1.1em] leading-none">√</span>
      <span className="border-t border-current pl-[0.15em] pr-[0.1em] pt-[0.12em]">
        <Node node={node.body} display={display} />
      </span>
    </span>
  );
}

function Matrix({ node }) {
  return (
    <span className="inline-flex items-center align-middle">
      {node.open ? (
        <span className="pr-[0.15em] text-[1.6em] leading-none">{node.open}</span>
      ) : null}
      <span
        className="inline-grid gap-x-[0.7em] gap-y-[0.15em]"
        style={{
          gridTemplateColumns: `repeat(${Math.max(...node.rows.map((row) => row.length), 1)}, auto)`,
        }}
      >
        {node.rows.map((row, rowIndex) =>
          row.map((cell, cellIndex) => (
            <span key={`${rowIndex}-${cellIndex}`} className="text-center">
              <Node node={cell} display={false} />
            </span>
          ))
        )}
      </span>
      {node.close ? (
        <span className="pl-[0.15em] text-[1.6em] leading-none">{node.close}</span>
      ) : null}
    </span>
  );
}

const STYLE_CLASS = {
  upright: "not-italic",
  bold: "font-semibold not-italic",
  italic: "italic",
  mono: "font-mono not-italic text-[0.92em]",
};

function Node({ node, display }) {
  if (!node) return null;
  switch (node.type) {
    case "row":
      return <Row node={node} display={display} />;
    case "text":
      return <span className={node.upright ? "not-italic" : "italic"}>{node.value}</span>;
    case "styled":
      return (
        <span className={STYLE_CLASS[node.style]}>
          <Node node={node.body} display={display} />
        </span>
      );
    case "frac":
      return <Fraction node={node} display={display} />;
    case "sqrt":
      return <Root node={node} display={display} />;
    case "script":
      return <Scripts node={node} display={display} />;
    case "accent":
      return (
        <span className="relative inline-block">
          <Node node={node.body} display={display} />
          <span aria-hidden="true" className="absolute inset-x-0 top-0 text-center leading-none">
            {node.accent}
          </span>
        </span>
      );
    case "space":
      return <span style={{ display: "inline-block", width: `${node.size}em` }} />;
    case "break":
      return <br />;
    case "matrix":
      return <Matrix node={node} display={display} />;
    default:
      return null;
  }
}

/**
 * `$...$` in a sentence, or a `$$...$$` block on its own.
 *
 * The source is kept in `title` so a formula that came out wrong can be read
 * as it was written, without a round trip through the model.
 */
export function MathSpan({ value, display = false }) {
  const tree = React.useMemo(() => parseMath(value), [value]);
  if (display) {
    return (
      <div
        className="my-3 overflow-x-auto no-scrollbar text-center text-[1.05em] italic"
        title={value}
      >
        <Node node={tree} display />
      </div>
    );
  }
  return (
    <span className="italic" title={value}>
      <Node node={tree} display={false} />
    </span>
  );
}
