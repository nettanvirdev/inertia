import * as React from "react";
import { CheckSquare, Square } from "@/components/icons";
import { cn } from "@/lib/utils";
import { parseInline } from "./parse-inline.js";
import { Caret, renderInline } from "./Inline.jsx";
import { CodeBlock } from "./CodeBlock.jsx";
import { MermaidBlock } from "./mermaid/MermaidBlock.jsx";
import { MathSpan } from "./math/Math.jsx";

/** Fence tags that mean "this is a diagram, not a program". */
const DIAGRAM = /^(mermaid|mmd)$/i;

/**
 * One block to one element.
 *
 * `caret` marks the block the cursor belongs to - it travels down to the last
 * text-bearing leaf so the caret lands at the end of the last line, not under
 * the block. Code, tables and rules never take it: a caret inside a table cell
 * or a copyable code body reads as content rather than as a cursor.
 */

const HEADING_SIZE = {
  1: "text-xl",
  2: "text-lg",
  3: "text-base",
  4: "text-base",
  5: "text-[15px]",
  6: "text-[15px] text-muted-foreground",
};

function Text({ text, caret }) {
  const nodes = React.useMemo(() => parseInline(text), [text]);
  return (
    <>
      {renderInline(nodes)}
      {caret ? <Caret /> : null}
    </>
  );
}

function Paragraph({ block, caret }) {
  return (
    <p className="my-2 first:mt-0 last:mb-0">
      <Text text={block.text} caret={caret} />
    </p>
  );
}

function Heading({ block, caret }) {
  const level = Math.min(Math.max(block.level, 1), 6);
  const Tag = `h${level}`;
  return (
    <Tag
      className={cn(
        "mt-4 mb-1.5 font-medium leading-snug text-heading first:mt-0",
        HEADING_SIZE[level]
      )}
    >
      <Text text={block.text} caret={caret} />
    </Tag>
  );
}

function Quote({ block, caret }) {
  return (
    <div className="my-3 rounded-2xl fill-whisper px-4 py-2.5 text-muted-foreground">
      <BlockList blocks={block.blocks} caret={caret} />
    </div>
  );
}

function List({ block, caret }) {
  const Tag = block.ordered ? "ol" : "ul";
  const isTask = block.items.some((item) => item.checked !== null);
  return (
    <Tag
      start={block.ordered && block.start !== 1 ? block.start : undefined}
      className={cn(
        "my-2 flex flex-col pl-5 marker:text-muted-foreground",
        block.tight ? "gap-1" : "gap-2.5",
        // a task list carries its own state glyph, so the marker would only
        // be a second bullet in front of it
        isTask ? "list-none pl-0.5" : block.ordered ? "list-decimal" : "list-disc"
      )}
    >
      {block.items.map((item, index) => {
        const last = caret && index === block.items.length - 1;
        const body = <BlockList blocks={item.blocks} caret={last} tight={block.tight} />;
        if (item.checked === null)
          return (
            <li key={index} className="pl-0.5">
              {body}
            </li>
          );
        return (
          <li key={index} className="flex items-baseline gap-2">
            {item.checked ? (
              <CheckSquare className="size-3.5 shrink-0 translate-y-[0.15em] text-muted-foreground" />
            ) : (
              <Square className="size-3.5 shrink-0 translate-y-[0.15em] text-muted-foreground" />
            )}
            <span className={cn("min-w-0", item.checked && "text-muted-foreground")}>{body}</span>
          </li>
        );
      })}
    </Tag>
  );
}

const CELL_ALIGN = { left: "text-left", center: "text-center", right: "text-right" };

function Table({ block }) {
  return (
    <div className="my-3 min-w-0 overflow-x-auto rounded-2xl fill-whisper">
      <table className="w-full min-w-max border-collapse text-[13.5px]">
        <thead>
          <tr className="fill-control">
            {block.head.map((cell, index) => (
              <th
                key={index}
                scope="col"
                className={cn(
                  "px-3 py-2 align-top font-medium text-foreground",
                  CELL_ALIGN[block.align[index]] ?? "text-left"
                )}
              >
                <Text text={cell} />
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {block.rows.map((row, rowIndex) => (
            <tr key={rowIndex}>
              {row.map((cell, index) => (
                <td
                  key={index}
                  className={cn(
                    "px-3 py-1.5 align-top text-muted-foreground",
                    CELL_ALIGN[block.align[index]] ?? "text-left"
                  )}
                >
                  <Text text={cell} />
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

/** Renders a run of blocks; only the last one can hold the caret. */
export function BlockList({ blocks, caret, tight = false }) {
  // A tight list item holding a single paragraph renders as bare inline
  // content, so the bullet sits on the text rather than above a block.
  if (tight && blocks.length === 1 && blocks[0].type === "paragraph")
    return <Text text={blocks[0].text} caret={caret} />;

  return blocks.map((block, index) => (
    <BlockView
      key={`${index}-${block.type}`}
      block={block}
      caret={caret && index === blocks.length - 1}
    />
  ));
}

function BlockView({ block, caret }) {
  switch (block.type) {
    case "code":
      return DIAGRAM.test(block.lang ?? "") ? (
        <MermaidBlock lang={block.lang} code={block.code} closed={block.closed} />
      ) : (
        <CodeBlock lang={block.lang} code={block.code} closed={block.closed} />
      );
    case "math":
      return <MathSpan value={block.value} display />;
    // The root pulls these out and renders them together at the foot of the
    // message; one nested inside a list or a quote is out of the root's reach,
    // so it renders where it stands rather than disappearing.
    case "footnote":
      return (
        <div className="my-1 flex gap-2 text-[0.92em] text-muted-foreground">
          <span className="shrink-0">[{block.id}]</span>
          <div className="min-w-0 flex-1">
            <BlockList blocks={block.blocks} tight />
          </div>
        </div>
      );
    case "hr":
      return <div role="separator" className="my-4 h-px bg-border/60" />;
    case "heading":
      return <Heading block={block} caret={caret} />;
    case "quote":
      return <Quote block={block} caret={caret} />;
    case "list":
      return <List block={block} caret={caret} />;
    case "table":
      return <Table block={block} />;
    default:
      return <Paragraph block={block} caret={caret} />;
  }
}

/**
 * Memoised at the block boundary. While a reply streams, the parse re-runs on
 * every token but every block object except the last is reused by identity
 * (see `Markdown.jsx`), so this comparison is a pointer check and React
 * re-renders exactly one block per token.
 */
export const MemoBlock = React.memo(BlockView);
