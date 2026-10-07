/**
 * The Inertia icon set - the single import for every glyph in the app.
 *
 *   import { Agent, ChevronDown, Sparkles } from "@/components/icons";
 *
 * Names match the shape of a conventional icon library on purpose: call sites
 * read the same as they always did, and swapping a glyph for a better drawing
 * is a change to one line of geometry rather than a change to every screen.
 *
 * The whole set is a few kilobytes of path data compiled at module load. It
 * replaced a third-party icon dependency that cost ~600KB of JS, and - more to
 * the point - it means the app's glyphs are drawn to the same design DNA as
 * its surfaces instead of borrowing someone else's.
 */
import { createIcons } from "./icon-base.jsx";
import { GLYPHS_CORE } from "./glyphs.core.js";
import { GLYPHS_FILES } from "./glyphs.files.js";
import { GLYPHS_PEOPLE } from "./glyphs.people.js";
import { GLYPHS_SYSTEM } from "./glyphs.system.js";
import { GLYPHS_STATUS } from "./glyphs.status.js";
import { GLYPHS_TOOLS } from "./glyphs.tools.js";
import { GLYPHS_SCIENCE } from "./glyphs.science.js";

export const GLYPHS = {
  ...GLYPHS_CORE,
  ...GLYPHS_FILES,
  ...GLYPHS_PEOPLE,
  ...GLYPHS_SYSTEM,
  ...GLYPHS_STATUS,
  ...GLYPHS_TOOLS,
  ...GLYPHS_SCIENCE,
};

const ICONS = createIcons(GLYPHS);

/** Every glyph name, sorted - used by the icon gallery and by `getIcon`. */
export const ICON_NAMES = Object.keys(ICONS).sort();

/**
 * Names that have already been complained about, so a bad name inside a list
 * that re-renders on every keystroke warns once rather than a thousand times.
 */
const warned = new Set();

/**
 * Whether to make an unknown name obvious. Vite defines `import.meta.env`; the
 * `?.` is for the test runner and anything else that loads this module without
 * one, where the answer is simply "no, behave like production".
 */
const LOUD = import.meta.env?.DEV === true;

/**
 * A cheap "close enough to suggest" test for the development warning.
 *
 * Not an edit distance - the two mistakes that actually happen are a dropped
 * or swapped letter ("Databse") and a plausible name from another icon library
 * ("GitPullRequest" for `GitBranch`). Same first three letters, or one name
 * containing the other, catches both and costs one pass over 162 strings on a
 * path that only runs when something is already wrong.
 */
function resembles(known, wanted) {
  const a = known.toLowerCase();
  const b = wanted.toLowerCase();
  return a.slice(0, 3) === b.slice(0, 3) || a.includes(b) || b.includes(a);
}

/**
 * Resolve a name coming from the data layer.
 *
 * A name is data - it comes from a JSON routine, a provider preset, a tool
 * catalog - so a typo is not a build error, and the old fallback to `Circle`
 * turned it into nothing at all: the routine icon picker offered a
 * "GitPullRequest" the set has never had, and drew a plain dot in its place.
 * A dot is a legitimate glyph, so nobody could tell the difference.
 *
 * In development an unknown name is now a warning and an `OctagonAlert`, which
 * no screen in the app draws for any other reason - so a stop sign on a screen
 * can only mean "that name does not exist", never "that drawing is wrong".
 * Production keeps the quiet dot, because a shipped app that shouts about its
 * own data is worse than one that renders a slightly wrong icon.
 */
export function getIcon(name) {
  const found = ICONS[name];
  if (found) return found;

  if (LOUD) {
    if (!warned.has(name)) {
      warned.add(name);
      // A near-miss list rather than all 162 names. The bug is almost always
      // a spelling, and printing the whole set buries the one line that
      // matters under a screenful of names nobody is looking for.
      const near = ICON_NAMES.filter((known) => resembles(known, String(name)));
      console.warn(
        `[icons] no glyph named ${JSON.stringify(name)}` +
          (near.length ? `. Did you mean ${near.join(", ")}?` : " - add it to a glyphs.*.js table.")
      );
    }
    return ICONS.OctagonAlert;
  }
  return ICONS.Circle;
}

/** `<Icon name="Globe" className="size-4" />` - for names that are data, not code. */
export function Icon({ name, ...props }) {
  const Cmp = getIcon(name);
  return <Cmp {...props} />;
}

export const {
  ArrowDown,
  ArrowUp,
  ArrowLeft,
  ArrowRight,
  Check,
  ChevronDown,
  ChevronUp,
  ChevronLeft,
  ChevronRight,
  ChevronsLeft,
  ChevronsRight,
  Circle,
  Square,
  Minus,
  Plus,
  X,
  MoreHorizontal,
  CircleAlert,
  CircleCheck,
  CircleX,
  OctagonAlert,
  TriangleAlert,
  Info,
  Eye,
  EyeOff,
  Search,
  SearchX,
  Command,
  Copy,
  Download,
  ExternalLink,
  Maximize2,
  RotateCw,
  RotateCcw,
  Play,
  Pause,
  Loader,
  Github,

  AudioLines,
  BookOpen,
  Camera,
  Code,
  Code2,
  Database,
  File,
  FileJson,
  FilePen,
  FileSearch,
  FileText,
  Folder,
  FolderCheck,
  FolderKanban,
  FolderOpen,
  FolderPlus,
  FolderTree,
  Frame,
  Image,
  Layers,
  LayoutGrid,
  Library,
  List,
  ListTodo,
  NotebookText,
  Package,
  PackagePlus,
  Rows3,
  ScrollText,
  Table,
  Video,
  Ticket,
  CheckSquare,
  SquareKanban,
  Box,

  Agent,
  AtSign,
  Brain,
  Contact,
  Inbox,
  Mail,
  MailPlus,
  MessageCircle,
  MessageCircleQuestion,
  MessageSquare,
  MessageSquarePlus,
  MessagesSquare,
  Mic,
  Phone,
  Send,
  SendArrow,
  ThumbsUp,
  ThumbsDown,
  User,
  UserPlus,
  UserRound,
  Users,
  Crown,
  Heart,
  Sparkles,
  Lightbulb,
  Binoculars,
  Hand,
  MousePointer2,
  MousePointerClick,
  Pin,
  PinOff,
  Pencil,
  Trash2,
  Eraser,

  Activity,
  Blocks,
  Cloud,
  Container,
  Cpu,
  CreditCard,
  Diamond,
  GitBranch,
  Globe,
  Hammer,
  HardDrive,
  KeyRound,
  Keyboard,
  Laptop,
  Monitor,
  MonitorPlay,
  Plug,
  PowerOff,
  Puzzle,
  Repeat,
  Rocket,
  Route,
  Scale,
  Server,
  ServerCog,
  SquareTerminal,
  Terminal,
  TerminalSquare,
  Workflow,
  Wrench,
  Wifi,

  Bell,
  BellOff,
  CalendarClock,
  Moon,
  Palette,
  Paperclip,
  Settings,
  Settings2,
  ShieldAlert,
  ShieldCheck,
  Siren,
  SlidersHorizontal,
  Sun,
  SunMoon,
  Sunrise,
  Timer,
  Waves,
  Wind,
  Zap,
  FlaskConical,
  Pickaxe,
  Sprout,
  Satellite,

  Diff,
  ListChecks,
  ShieldQuestion,

  Sigma,
  Atom,
  Molecule,
  Orbit,
  Dna,
  BarChart,
} = ICONS;
