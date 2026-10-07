import * as React from "react";
import { cn } from "@/lib/utils";

/**
 * A diagram, drawn.
 *
 * SVG built from the geometry `layout.js` computed - no HTML from the model
 * ever reaches the DOM, the way nothing else in a message does. Colours come
 * from the palette through `currentColor` and CSS variables, so a diagram
 * follows the theme instead of arriving as a white rectangle at night.
 */

const SHAPE_RADIUS = { round: 12, stadium: 999, rect: 6, subroutine: 6, flag: 6 };

function nodePath(node) {
  const { x, y, w, h } = node;
  switch (node.shape) {
    case "diamond":
      return `M ${x + w / 2} ${y} L ${x + w} ${y + h / 2} L ${x + w / 2} ${y + h} L ${x} ${y + h / 2} Z`;
    case "hexagon": {
      const cut = Math.min(16, w / 4);
      return `M ${x + cut} ${y} L ${x + w - cut} ${y} L ${x + w} ${y + h / 2} L ${x + w - cut} ${y + h} L ${x + cut} ${y + h} L ${x} ${y + h / 2} Z`;
    }
    case "parallelogram": {
      const skew = Math.min(14, w / 5);
      return `M ${x + skew} ${y} L ${x + w} ${y} L ${x + w - skew} ${y + h} L ${x} ${y + h} Z`;
    }
    case "flag":
      return `M ${x} ${y} L ${x + w - 12} ${y} L ${x + w} ${y + h / 2} L ${x + w - 12} ${y + h} L ${x} ${y + h} Z`;
    default:
      return null;
  }
}

function NodeShape({ node }) {
  // Presentation attributes rather than classes: `fill-*` is this app's own
  // utility family, and a diagram borrowing that prefix for a stock Tailwind
  // colour is exactly the collision the utilities test exists to catch.
  const common = { fill: "var(--diagram-node)", stroke: "var(--diagram-line)", strokeWidth: 1 };
  if (node.shape === "circle") {
    return <circle cx={node.cx} cy={node.cy} r={Math.min(node.w, node.h) / 2} {...common} />;
  }
  if (node.shape === "cylinder") {
    const lip = 7;
    return (
      <g {...common}>
        <path
          d={`M ${node.x} ${node.y + lip} A ${node.w / 2} ${lip} 0 0 1 ${node.x + node.w} ${node.y + lip} L ${node.x + node.w} ${node.y + node.h - lip} A ${node.w / 2} ${lip} 0 0 1 ${node.x} ${node.y + node.h - lip} Z`}
        />
        <path
          d={`M ${node.x} ${node.y + lip} A ${node.w / 2} ${lip} 0 0 0 ${node.x + node.w} ${node.y + lip}`}
          fill="none"
          stroke="var(--diagram-line)"
        />
      </g>
    );
  }
  if (node.shape === "subroutine") {
    return (
      <g {...common}>
        <rect x={node.x} y={node.y} width={node.w} height={node.h} rx={6} />
        <line x1={node.x + 7} y1={node.y} x2={node.x + 7} y2={node.y + node.h} />
        <line x1={node.x + node.w - 7} y1={node.y} x2={node.x + node.w - 7} y2={node.y + node.h} />
      </g>
    );
  }
  const path = nodePath(node);
  if (path) return <path d={path} {...common} />;
  return (
    <rect
      x={node.x}
      y={node.y}
      width={node.w}
      height={node.h}
      rx={SHAPE_RADIUS[node.shape] ?? 6}
      {...common}
    />
  );
}

/** A label, centred, one `<tspan>` per line. */
function Label({ lines, x, y, className }) {
  const rows = lines?.length ? lines : [""];
  const first = y - ((rows.length - 1) * 15) / 2;
  return (
    <text
      x={x}
      y={first}
      textAnchor="middle"
      dominantBaseline="central"
      fill="var(--foreground)"
      className={cn("text-[11.5px]", className)}
    >
      {rows.map((line, index) => (
        <tspan key={index} x={x} dy={index === 0 ? 0 : 15}>
          {line}
        </tspan>
      ))}
    </text>
  );
}

const DASH = { dotted: "3 3", thick: undefined, solid: undefined, dashed: "5 4" };

function Flowchart({ diagram }) {
  return (
    <>
      {diagram.groups.map((group) => (
        <g key={group.id}>
          <rect
            x={group.x}
            y={group.y}
            width={group.w}
            height={group.h}
            rx={12}
            fill="var(--diagram-group)"
            stroke="var(--diagram-line)"
            strokeWidth={1}
            strokeDasharray="4 4"
          />
          <text
            x={group.x + 12}
            y={group.y + 15}
            fill="var(--muted-foreground)"
            className="text-[11px]"
          >
            {group.label}
          </text>
        </g>
      ))}

      {diagram.edges.map((edge, index) => (
        <g key={index}>
          <path
            d={edge.path}
            fill="none"
            stroke="var(--diagram-line)"
            strokeWidth={edge.stroke === "thick" ? 2.2 : 1.3}
            strokeDasharray={DASH[edge.stroke]}
            markerEnd={
              edge.head === "end" || edge.head === "both" ? "url(#inertia-arrow)" : undefined
            }
            markerStart={edge.head === "both" ? "url(#inertia-arrow-back)" : undefined}
          />
          {edge.head === "circle" ? (
            <circle
              cx={edge.endX}
              cy={edge.endY}
              r={4}
              fill="var(--diagram-node)"
              stroke="var(--diagram-line)"
            />
          ) : null}
          {edge.label ? (
            <>
              <rect
                x={edge.labelX - (edge.label.length * 6.9) / 2 - 4}
                y={edge.labelY - 9}
                width={edge.label.length * 6.9 + 8}
                height={18}
                rx={5}
                fill="var(--card-subtle)"
              />
              <Label
                lines={[edge.label]}
                x={edge.labelX}
                y={edge.labelY}
                className="text-[11px]"
                fill="var(--muted-foreground)"
              />
            </>
          ) : null}
        </g>
      ))}

      {diagram.nodes.map((node) => (
        <g key={node.id}>
          <NodeShape node={node} />
          <Label lines={node.lines} x={node.cx} y={node.cy} />
        </g>
      ))}
    </>
  );
}

function Sequence({ diagram }) {
  return (
    <>
      {diagram.actors.map((actor) => (
        <g key={actor.id}>
          <line
            x1={actor.cx}
            y1={diagram.lifelineTop}
            x2={actor.cx}
            y2={diagram.lifelineBottom}
            stroke="var(--diagram-line)"
            strokeWidth={1}
            strokeDasharray="4 4"
          />
          {[actor.y, actor.footY].map((top, index) => (
            <g key={index}>
              <rect
                x={actor.cx - actor.w / 2}
                y={top}
                width={actor.w}
                height={actor.h}
                rx={8}
                fill="var(--diagram-node)"
                stroke="var(--diagram-line)"
                strokeWidth={1}
              />
              <Label lines={[actor.label]} x={actor.cx} y={top + actor.h / 2} />
            </g>
          ))}
        </g>
      ))}

      {diagram.steps.map((step, index) => {
        if (step.type === "note") {
          return (
            <g key={index}>
              <rect
                x={step.x}
                y={step.y}
                width={step.w}
                height={step.h}
                rx={8}
                fill="var(--diagram-note)"
                stroke="var(--diagram-line)"
                strokeWidth={1}
              />
              <Label lines={step.lines} x={step.x + step.w / 2} y={step.y + step.h / 2} />
            </g>
          );
        }
        if (step.type === "block") {
          return (
            <text
              key={index}
              x={8}
              y={step.y + 12}
              fill="var(--muted-foreground)"
              className="text-[11px]"
            >
              {step.keyword}
              {step.label ? ` ${step.label}` : ""}
            </text>
          );
        }

        if (step.from === step.to) {
          const right = step.fromX + 46;
          return (
            <g key={index}>
              <path
                d={`M ${step.fromX} ${step.y} L ${right} ${step.y} L ${right} ${step.y + step.selfHeight} L ${step.fromX + 4} ${step.y + step.selfHeight}`}
                fill="none"
                stroke="var(--diagram-line)"
                strokeWidth={1.3}
                strokeDasharray={DASH[step.stroke]}
                markerEnd={step.head ? "url(#inertia-arrow)" : undefined}
              />
              <text
                x={right + 8}
                y={step.y + step.selfHeight / 2}
                dominantBaseline="central"
                fill="var(--foreground)"
                className="text-[11.5px]"
              >
                {step.label}
              </text>
            </g>
          );
        }

        const forward = step.toX > step.fromX;
        const startX = step.fromX + (forward ? 2 : -2);
        const endX = step.toX + (forward ? -8 : 8);
        return (
          <g key={index}>
            <line
              x1={startX}
              y1={step.y}
              x2={endX}
              y2={step.y}
              stroke="var(--diagram-line)"
              strokeWidth={1.3}
              strokeDasharray={DASH[step.stroke]}
              markerEnd={step.head ? "url(#inertia-arrow)" : undefined}
            />
            <Label
              lines={step.lines}
              x={(step.fromX + step.toX) / 2}
              y={step.y - 10 - ((step.lines.length - 1) * 15) / 2}
            />
          </g>
        );
      })}
    </>
  );
}

export function Diagram({ diagram, className }) {
  const { viewBox } = diagram;
  return (
    <svg
      viewBox={`${viewBox.x} ${viewBox.y} ${viewBox.w} ${viewBox.h}`}
      width={diagram.width}
      height={diagram.height}
      role="img"
      className={cn("max-w-full", className)}
    >
      <defs>
        <marker
          id="inertia-arrow"
          viewBox="0 0 10 10"
          refX="9"
          refY="5"
          markerWidth="7"
          markerHeight="7"
          orient="auto-start-reverse"
        >
          <path d="M 0 1 L 10 5 L 0 9 z" fill="var(--diagram-line)" />
        </marker>
        <marker
          id="inertia-arrow-back"
          viewBox="0 0 10 10"
          refX="1"
          refY="5"
          markerWidth="7"
          markerHeight="7"
          orient="auto-start-reverse"
        >
          <path d="M 10 1 L 0 5 L 10 9 z" fill="var(--diagram-line)" />
        </marker>
      </defs>
      {diagram.kind === "flowchart" ? (
        <Flowchart diagram={diagram} />
      ) : (
        <Sequence diagram={diagram} />
      )}
    </svg>
  );
}
