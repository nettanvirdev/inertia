/**
 * Which tab is in front, and what order they sit in.
 *
 * Small enough to look obvious and worth its own file anyway, because every
 * rule here is one somebody would otherwise get subtly wrong inside a
 * component: an order that silently drops a new tab, an active id that points
 * at a pane which has been closed, a reorder that is off by one when something
 * moves rightwards.
 */

/**
 * Sections in the order the person dragged them into.
 *
 * The stored order is advisory, not authoritative. It is a list of ids from
 * some earlier session and it will routinely be missing ids that exist now - a
 * pane added in an update - and holding ids that do not - a pane removed. So
 * known ids are placed in their remembered position and everything else keeps
 * its natural order at the end, which means a new pane appears rather than
 * silently not appearing.
 */
export function orderTabs(sections = [], order = []) {
  const list = Array.isArray(sections) ? sections.filter(Boolean) : [];
  const wanted = Array.isArray(order) ? order : [];
  const known = new Set(wanted);

  const placed = wanted
    .map((id) => list.find((section) => section.id === id))
    .filter(Boolean);
  const rest = list.filter((section) => !known.has(section.id));
  return [...placed, ...rest];
}

/**
 * The tab to show.
 *
 * `wanted` is what the person last picked, and it is checked against what is
 * actually open rather than trusted: closing the front tab has to hand the
 * front to something, and a dock whose active id points at a closed pane
 * renders an empty column with a tab strip above it.
 */
export function activeTab(open = [], wanted = null) {
  const list = Array.isArray(open) ? open.filter(Boolean) : [];
  if (!list.length) return null;
  const found = list.find((section) => section.id === wanted);
  return (found ?? list[0]).id;
}

/**
 * An order with one id moved to sit before another.
 *
 * Expressed as "put `moved` where `before` is" rather than as two indices,
 * because that is what a drop actually knows: the tab under the pointer. The
 * moved id is removed first and then inserted, so moving something rightwards
 * lands where it looks like it will - removing after inserting is the off-by-
 * one every list reorder is born with.
 *
 * `before` of null means the end.
 */
export function moveTab(order = [], moved, before = null) {
  const list = (Array.isArray(order) ? order : []).filter((id) => id !== moved);
  if (!moved) return list;
  if (before == null) return [...list, moved];
  const at = list.indexOf(before);
  if (at === -1) return [...list, moved];
  return [...list.slice(0, at), moved, ...list.slice(at)];
}
