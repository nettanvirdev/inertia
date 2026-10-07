import React from "react";
import { AvatarGroup } from "@/components/ui/avatar";
import { AgentAvatar } from "@/features/agents/AgentAvatar";
import { Tooltip } from "@/components/ui/tooltip";
import { othersIn } from "./room.js";

/*
 * Who else is in this conversation.
 *
 * A group thread has one agent speaking and several in the room, and the
 * header can only carry one name. This is the rest of them: small, beside the
 * name, and doing nothing but saying who is here - which turns out to be the
 * whole question a person has when a reply arrives under a name they were not
 * expecting.
 *
 * Nothing here says whose turn it is. The transcript already answers that,
 * with a bubble that is either being written or is not.
 */

export function RoomStrip({ room, agents = [], speaking }) {
  const others = othersIn(room, agents, speaking);
  if (others.length === 0) return null;

  const names = others.map((one) => one.name).join(", ");

  return (
    <Tooltip content={`Also here: ${names}`}>
      <div
        className="hidden shrink-0 items-center md:flex"
        aria-label={`Also in this conversation: ${names}`}
      >
        <AvatarGroup size="xs" max={4}>
          {others.map((one) => (
            <AgentAvatar key={one.id} agent={one} size="xs" />
          ))}
        </AvatarGroup>
      </div>
    </Tooltip>
  );
}

/**
 * The line that appears when the agents have been talking for a while.
 *
 * The hop limit exists because five agents that can each call the next have no
 * natural end, and the bill is real. When it stops them, the person is the one
 * who has to say so - and they can only do that if they are told, in the place
 * they are about to type.
 */
export function FloorNote({ room }) {
  if (room?.reason !== "hop-limit") return null;
  return (
    <p className="px-1 pb-1 text-[11px] text-muted-foreground">
      The agents have been talking among themselves for a few turns. Say something to carry on.
    </p>
  );
}
