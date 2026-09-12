import * as React from "react";
import { Check, Code, Copy, Maximize2 } from "@/components/icons";
import { IconButton } from "@/components/ui/icon-button";
import { copyText } from "@/lib/clipboard";
import { CodeBlock } from "../CodeBlock.jsx";
import { parseMermaid } from "./parse.js";
import { layoutDiagram } from "./layout.js";
import { Diagram } from "./Diagram.jsx";

/**
 * A `mermaid` fence, as a picture where that is possible.
 *
 * Three things decide what this renders, in order:
 *
 *  - A fence still streaming is code. Half a diagram redrawn on every token is
 *    a diagram that jumps around the message while it is being written, and
 *    the source arriving line by line is the honest thing to show meanwhile.
 *  - A diagram this cannot read is code. `parseMermaid` returning null is the
 *    ordinary answer for a class diagram or a Gantt chart, and the fence then
 *    renders exactly as it did before diagrams existed - never an error.
 *  - Anything else is drawn, with its source one click away, because a picture
 *    the reader cannot check against the text is a picture they have to trust.
 */
export function MermaidBlock({ code, lang, closed = true }) {
  const [showSource, setShowSource] = React.useState(false);
  const [zoomed, setZoomed] = React.useState(false);
  const [copied, setCopied] = React.useState(false);
  const timer = React.useRef(0);

  React.useEffect(() => () => window.clearTimeout(timer.current), []);

  const diagram = React.useMemo(() => {
    if (!closed) return null;
    return layoutDiagram(parseMermaid(code));
  }, [code, closed]);

  if (!diagram) return <CodeBlock lang={lang} code={code} />;

  const copy = async () => {
    if (!(await copyText(code))) return;
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 1400);
  };

  return (
    <div className="my-3 min-w-0 overflow-hidden rounded-2xl card-surface-subtle">
      <div className="flex h-8 items-center justify-between pl-3 pr-1.5">
        <span className="truncate text-[11px] text-muted-foreground">
          {diagram.kind === "sequence" ? "sequence diagram" : "diagram"}
        </span>
        <div className="flex items-center gap-0.5">
          <IconButton
            size="sm"
            label={zoomed ? "Fit to the message" : "Show it at full size"}
            active={zoomed}
            onClick={() => setZoomed((value) => !value)}
          >
            <Maximize2 />
          </IconButton>
          <IconButton
            size="sm"
            label={showSource ? "Show the diagram" : "Show the source"}
            active={showSource}
            onClick={() => setShowSource((value) => !value)}
          >
            <Code />
          </IconButton>
          <IconButton size="sm" label="Copy the source" onClick={copy}>
            {copied ? <Check className="text-success-ink" /> : <Copy />}
          </IconButton>
        </div>
      </div>

      {showSource ? (
        <pre className="no-scrollbar overflow-x-auto px-3 pb-3 pt-0.5">
          <code className="font-mono text-[12.5px] leading-relaxed text-foreground">{code}</code>
        </pre>
      ) : (
        <div className="no-scrollbar overflow-x-auto px-3 pb-3 pt-0.5">
          {/* Shrunk to fit the column by default, never stretched: a diagram
              blown up to the width of the message has type twice the size of
              the prose around it. Full size is one click away for the wide one
              that is worth scrolling. */}
          <Diagram diagram={diagram} className={zoomed ? "max-w-none" : "h-auto max-w-full"} />
        </div>
      )}
    </div>
  );
}
