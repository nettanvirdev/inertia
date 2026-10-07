//! Keeping the screen, not the stream.
//!
//! A pty does not emit text. It emits instructions for painting a grid, and on
//! Windows in particular it does so by absolute position: ConPTY does not
//! stream lines, it repaints the visible viewport, moving the cursor to a row
//! and column and writing over what was there. Which means the obvious
//! approach, keeping the bytes and stripping the escape sequences when somebody
//! asks for text, does not merely lose the colours. It loses the words.
//!
//! That is not a theory. It is what the Electron app did first, and its smoke
//! test caught it: `echo INERTIA_MARKER_OK` ran, the marker was printed, and
//! reading the terminal back produced fifteen blank lines and a prompt. The
//! text had been written into a grid and then overwritten in place, and a
//! stripper with no grid has no way to know what survived.
//!
//! So this is a grid. It is deliberately small - the sequences ConPTY and an
//! ordinary shell prompt actually emit, and no more - because what it feeds is
//! `terminal_read`, which wants the words a person can see rather than a
//! faithful rendering of every attribute. Colour, blinking and the alternate
//! screen buffer are parsed and thrown away on purpose: the window has a real
//! emulator (xterm.js) fed by the same bytes, and that is where fidelity
//! belongs.
//!
//! ## Why the raw bytes are kept as well
//!
//! A pane reopening replays into xterm.js, which wants the byte stream, not
//! this text. Regenerating a stream from the grid needs a serializer this crate
//! does not have, so `session.rs` keeps a bounded tail of the raw bytes beside
//! the grid and replays that. The cost is that anything scrolled out of the
//! byte tail is gone from a replay; the grid is what `terminal_read` reads and
//! it is not affected.

/// How many lines of history the grid keeps.
///
/// The ceiling on what one terminal costs the process: a line is a row of
/// cells, so this is the number that decides whether a shell left printing
/// overnight is a few megabytes or a few hundred.
pub const SCROLLBACK_LINES: usize = 5000;

/// Where the parser is, between chunks.
///
/// An escape sequence is split across two reads as often as not - a pty is a
/// byte stream and has no idea about sequences - so the state has to survive a
/// call. The first version parsed each chunk from `Ground` and left a stray
/// `[0m` in the middle of a sentence every time a read landed mid-escape.
#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Ground,
    /// `ESC` seen.
    Escape,
    /// `ESC [` seen; the accumulated parameter and intermediate bytes.
    Csi(String),
    /// `ESC ]` seen; the payload so far. The payload itself is read by
    /// `ansi::read_cwd` from the raw chunk, so it is only skipped here.
    Osc,
    /// Inside an OSC and an `ESC` has arrived: the next byte decides whether
    /// that was the ST terminator or the start of something else.
    OscEscape,
}

/// A terminal's screen: the scrollback, the viewport, and a cursor in it.
#[derive(Debug)]
pub struct Screen {
    cols: usize,
    rows: usize,
    /// Every line, scrollback first. The viewport is `lines[top..top + rows]`.
    lines: Vec<Vec<char>>,
    top: usize,
    /// Cursor row WITHIN the viewport, and column.
    row: usize,
    col: usize,
    saved: (usize, usize),
    state: State,
    /// What the shell asked this emulator and is waiting to hear back.
    ///
    /// A terminal is not only a thing that draws. ConPTY opens by asking where
    /// the cursor is - `ESC [ 6 n`, a Device Status Report - and does not print
    /// a single byte until something answers. The first version of this crate
    /// drew a beautiful empty screen for exactly that reason: the shell had
    /// started, the prompt was ready, and it was politely waiting for a reply
    /// nobody was ever going to send.
    replies: String,
    /// Turn a bare `\n` into `\r\n`.
    ///
    /// For anything that is not a pty: a piped process writes `\n` where a
    /// terminal writes `\r\n`, and without this every line would start where
    /// the last one ended and the whole screen would be one long diagonal.
    convert_eol: bool,
}

impl Screen {
    pub fn new(cols: usize, rows: usize, convert_eol: bool) -> Self {
        let cols = cols.clamp(1, 1000);
        let rows = rows.clamp(1, 1000);
        Self {
            cols,
            rows,
            lines: vec![Vec::new(); rows],
            top: 0,
            row: 0,
            col: 0,
            saved: (0, 0),
            state: State::Ground,
            replies: String::new(),
            convert_eol,
        }
    }

    /// Anything the shell asked for and is waiting on, to be written back into
    /// the pty. Empty almost always; never ignorable when it is not.
    pub fn take_replies(&mut self) -> String {
        std::mem::take(&mut self.replies)
    }

    /// Feed the emulator, one chunk of whatever the shell said.
    ///
    /// Synchronous, unlike the Electron version: xterm.js queues a write and
    /// parses it on a later tick, so that app needed a `settle()` before every
    /// read or `terminal_read` returned the screen as it was BEFORE the command
    /// it was asked about. Here the grid is up to date the moment this returns,
    /// and there is nothing to wait for.
    pub fn write(&mut self, text: &str) {
        for ch in text.chars() {
            self.feed(ch);
        }
    }

    fn feed(&mut self, ch: char) {
        match std::mem::replace(&mut self.state, State::Ground) {
            State::Ground => self.ground(ch),
            State::Escape => match ch {
                '[' => self.state = State::Csi(String::new()),
                ']' => self.state = State::Osc,
                // DCS, PM, APC: a payload terminated by ST. Rare from a shell,
                // and read as an OSC here because both end the same way.
                'P' | '^' | '_' => self.state = State::Osc,
                // Reverse index: up a line, scrolling the top if there is no
                // line above. Emitted by anything that redraws upward.
                'M' => self.reverse_index(),
                '7' => self.saved = (self.row, self.col),
                '8' => {
                    self.row = self.saved.0.min(self.rows - 1);
                    self.col = self.saved.1.min(self.cols);
                }
                // Charset selection: `ESC ( B` and friends, two bytes of which
                // this is the first.
                '(' | ')' | '#' | '%' => self.state = State::Escape,
                _ => {}
            },
            State::Csi(mut params) => {
                if ('\u{40}'..='\u{7e}').contains(&ch) {
                    self.csi(&params, ch);
                } else {
                    params.push(ch);
                    // A runaway sequence is a sequence that was never one. Bail
                    // out rather than swallowing the rest of a build's output
                    // into a parameter string.
                    self.state = if params.len() > 64 {
                        State::Ground
                    } else {
                        State::Csi(params)
                    };
                }
            }
            State::Osc => match ch {
                '\u{7}' => {}
                '\u{1b}' => self.state = State::OscEscape,
                _ => self.state = State::Osc,
            },
            State::OscEscape => {
                // `ESC \` closed it; anything else was a new escape, and this
                // is its first byte.
                if ch != '\\' {
                    self.state = State::Escape;
                    self.feed(ch);
                }
            }
        }
    }

    fn ground(&mut self, ch: char) {
        match ch {
            '\u{1b}' => self.state = State::Escape,
            '\r' => self.col = 0,
            '\n' => {
                if self.convert_eol {
                    self.col = 0;
                }
                self.newline();
            }
            '\t' => {
                // Tab stops every eight columns, which is what every shell
                // assumes when it lays out a listing.
                let next = (self.col / 8 + 1) * 8;
                self.col = next.min(self.cols.saturating_sub(1));
            }
            '\u{8}' => self.col = self.col.saturating_sub(1),
            // Bell, and the rest of the C0 set. No text and no layout.
            c if (c as u32) < 0x20 || c == '\u{7f}' => {}
            c => self.put(c),
        }
    }

    fn put(&mut self, ch: char) {
        if self.col >= self.cols {
            self.col = 0;
            self.newline();
        }
        let col = self.col;
        let line = self.line_mut();
        while line.len() <= col {
            line.push(' ');
        }
        line[col] = ch;
        self.col += 1;
    }

    fn line_mut(&mut self) -> &mut Vec<char> {
        let at = self.top + self.row;
        while self.lines.len() <= at {
            self.lines.push(Vec::new());
        }
        &mut self.lines[at]
    }

    fn newline(&mut self) {
        if self.row + 1 < self.rows {
            self.row += 1;
        } else {
            self.top += 1;
            self.trim();
        }
        let want = self.top + self.rows;
        while self.lines.len() < want {
            self.lines.push(Vec::new());
        }
    }

    fn reverse_index(&mut self) {
        if self.row > 0 {
            self.row -= 1;
        } else if self.top > 0 {
            self.top -= 1;
        } else {
            self.lines.insert(0, Vec::new());
        }
    }

    /// Drop the oldest history once there is more of it than anyone will read.
    fn trim(&mut self) {
        if self.top > SCROLLBACK_LINES {
            let drop = self.top - SCROLLBACK_LINES;
            self.lines.drain(0..drop);
            self.top -= drop;
        }
    }

    fn csi(&mut self, params: &str, final_byte: char) {
        // `ESC [ ? ...` is a private sequence: the alternate screen buffer,
        // cursor visibility, bracketed paste. None of them move text around in
        // a way this grid can honour, and pretending to would be worse than
        // ignoring them - a half-implemented alt screen shows a person's vim
        // session interleaved with their prompt.
        if params.starts_with('?') || params.starts_with('>') {
            // Except the one it is waiting on an answer for. `ESC [ ? 6 n` is
            // the extended cursor report, and a shell that asked for it stops
            // until it gets one.
            if final_byte == 'n' && params.trim_start_matches('?') == "6" {
                self.replies
                    .push_str(&format!("\u{1b}[{};{}R", self.row + 1, self.col + 1));
            }
            return;
        }
        let numbers: Vec<usize> = params
            .split(';')
            .map(|part| part.trim().parse::<usize>().unwrap_or(0))
            .collect();
        let at = |i: usize| numbers.get(i).copied().unwrap_or(0);
        let one = |i: usize| at(i).max(1);

        match final_byte {
            // Cursor up, down, forward, back.
            'A' => self.row = self.row.saturating_sub(one(0)),
            'B' | 'e' => self.row = (self.row + one(0)).min(self.rows - 1),
            'C' | 'a' => self.col = (self.col + one(0)).min(self.cols - 1),
            'D' => self.col = self.col.saturating_sub(one(0)),
            // Next line, previous line: down or up, and to column zero.
            'E' => {
                self.row = (self.row + one(0)).min(self.rows - 1);
                self.col = 0;
            }
            'F' => {
                self.row = self.row.saturating_sub(one(0));
                self.col = 0;
            }
            // Absolute column, absolute row, and both at once.
            'G' | '`' => self.col = (one(0) - 1).min(self.cols - 1),
            'd' => self.row = (one(0) - 1).min(self.rows - 1),
            'H' | 'f' => {
                self.row = (one(0) - 1).min(self.rows - 1);
                self.col = (one(1) - 1).min(self.cols - 1);
            }
            // Erase in display. This is the one ConPTY leans on hardest.
            'J' => self.erase_display(at(0)),
            'K' => self.erase_line(at(0)),
            'L' => self.insert_lines(one(0)),
            'M' => self.delete_lines(one(0)),
            'P' => self.delete_chars(one(0)),
            'X' => self.erase_chars(one(0)),
            '@' => self.insert_chars(one(0)),
            // The questions a terminal has to answer. Nothing is printed until
            // they are: see `replies`.
            'n' => match at(0) {
                5 => self.replies.push_str("\u{1b}[0n"),
                6 => self
                    .replies
                    .push_str(&format!("\u{1b}[{};{}R", self.row + 1, self.col + 1)),
                _ => {}
            },
            // "What kind of terminal are you?" Answered as a plain VT100 with
            // an Advanced Video Option, which is what xterm's own minimal reply
            // says and is true of this grid: text and cursor movement, nothing
            // that needs negotiating.
            'c' => self.replies.push_str("\u{1b}[?1;2c"),
            's' => self.saved = (self.row, self.col),
            'u' => {
                self.row = self.saved.0.min(self.rows - 1);
                self.col = self.saved.1.min(self.cols);
            }
            // Scroll up and down, by lines. Treated as a plain scroll of the
            // whole viewport, which is what a shell means by it.
            'S' => {
                for _ in 0..one(0) {
                    self.top += 1;
                }
                self.trim();
                let want = self.top + self.rows;
                while self.lines.len() < want {
                    self.lines.push(Vec::new());
                }
            }
            // Colour and everything else: parsed so it does not become text,
            // and then dropped. See the module doc.
            _ => {}
        }
    }

    fn viewport(&mut self, row: usize) -> Option<&mut Vec<char>> {
        let at = self.top + row;
        while self.lines.len() <= at {
            self.lines.push(Vec::new());
        }
        self.lines.get_mut(at)
    }

    fn erase_display(&mut self, mode: usize) {
        match mode {
            // To the end of the screen, from the cursor.
            0 => {
                self.erase_line(0);
                for row in self.row + 1..self.rows {
                    if let Some(line) = self.viewport(row) {
                        line.clear();
                    }
                }
            }
            1 => {
                self.erase_line(1);
                for row in 0..self.row {
                    if let Some(line) = self.viewport(row) {
                        line.clear();
                    }
                }
            }
            // The whole viewport. NOT the scrollback: `clear` in a shell sends
            // this and a person who then scrolls up expects their history.
            2 => {
                for row in 0..self.rows {
                    if let Some(line) = self.viewport(row) {
                        line.clear();
                    }
                }
            }
            // And this one IS the scrollback, which is what `clear` sends
            // second on a terminal that supports it.
            3 => {
                self.lines.drain(0..self.top);
                self.top = 0;
            }
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: usize) {
        let (row, col, cols) = (self.row, self.col, self.cols);
        let Some(line) = self.viewport(row) else {
            return;
        };
        match mode {
            0 => line.truncate(col),
            1 => {
                for i in 0..col.min(line.len()) {
                    line[i] = ' ';
                }
            }
            2 => line.clear(),
            _ => {}
        }
        let _ = cols;
    }

    fn erase_chars(&mut self, count: usize) {
        let (row, col) = (self.row, self.col);
        let Some(line) = self.viewport(row) else {
            return;
        };
        for i in col..(col + count).min(line.len()) {
            line[i] = ' ';
        }
    }

    fn delete_chars(&mut self, count: usize) {
        let (row, col) = (self.row, self.col);
        let Some(line) = self.viewport(row) else {
            return;
        };
        let end = (col + count).min(line.len());
        if col < line.len() {
            line.drain(col..end);
        }
    }

    fn insert_chars(&mut self, count: usize) {
        let (row, col, cols) = (self.row, self.col, self.cols);
        let Some(line) = self.viewport(row) else {
            return;
        };
        while line.len() < col {
            line.push(' ');
        }
        for _ in 0..count {
            line.insert(col, ' ');
        }
        line.truncate(cols);
    }

    fn insert_lines(&mut self, count: usize) {
        let at = self.top + self.row;
        for _ in 0..count.min(self.rows) {
            if self.lines.len() > self.top + self.rows {
                self.lines.remove(self.top + self.rows);
            }
            self.lines.insert(at.min(self.lines.len()), Vec::new());
        }
    }

    fn delete_lines(&mut self, count: usize) {
        let at = self.top + self.row;
        for _ in 0..count.min(self.rows) {
            if at < self.lines.len() {
                self.lines.remove(at);
            }
            let want = self.top + self.rows;
            while self.lines.len() < want {
                self.lines.push(Vec::new());
            }
        }
    }

    /// A new size, without reflowing what is already on screen.
    ///
    /// A real emulator re-wraps its history when the window narrows. This does
    /// not, and the difference shows up as an old line that stays long after a
    /// resize - which is invisible to the window, because the window has its
    /// own emulator and re-wraps there.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols.clamp(1, 1000);
        self.rows = rows.clamp(1, 1000);
        self.row = self.row.min(self.rows - 1);
        self.col = self.col.min(self.cols - 1);
        let want = self.top + self.rows;
        while self.lines.len() < want {
            self.lines.push(Vec::new());
        }
    }

    /// The screen as a person would read it, oldest line first.
    ///
    /// Trailing blanks come off each line and trailing blank lines come off the
    /// end: a grid is padded to its width and height, and without this every
    /// line would be a hundred characters wide and every read would end with
    /// thirty empty rows.
    pub fn text(&self) -> String {
        let mut out: Vec<String> = self
            .lines
            .iter()
            .map(|line| {
                let mut text: String = line.iter().collect();
                while text.ends_with(' ') {
                    text.pop();
                }
                text
            })
            .collect();
        while out.last().is_some_and(|line| line.trim().is_empty()) {
            out.pop();
        }
        out.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Screen {
        Screen::new(20, 5, false)
    }

    #[test]
    fn plain_output_reads_back_as_itself() {
        let mut s = screen();
        s.write("hello\r\nworld\r\n");
        assert_eq!(s.text(), "hello\nworld");
    }

    /// The bug this whole file exists for: ConPTY repaints by absolute
    /// position, so the words are written and then moved over. A stripper with
    /// no grid returns blank rows where the output was.
    #[test]
    fn a_repaint_by_absolute_position_keeps_the_words() {
        let mut s = screen();
        s.write("\u{1b}[2J\u{1b}[1;1HPS D:\\> echo hi\u{1b}[2;1Hhi\u{1b}[3;1HPS D:\\> ");
        // The trailing space of the last prompt comes off with the padding a
        // grid is filled with; there is no way to tell one from the other.
        assert_eq!(s.text(), "PS D:\\> echo hi\nhi\nPS D:\\>");
    }

    #[test]
    fn colour_never_reaches_the_text() {
        let mut s = screen();
        s.write("\u{1b}[32m\u{1b}[1mPASS\u{1b}[0m suite\r\n");
        assert_eq!(s.text(), "PASS suite");
    }

    /// Every spinner in the world redraws with a bare carriage return, and a
    /// two-minute install that reads back as four thousand copies of one
    /// sentence is a context window spent on nothing.
    #[test]
    fn a_line_redrawn_in_place_is_one_line() {
        let mut s = screen();
        s.write("installing 10%\rinstalling 90%\rinstalling done");
        assert_eq!(s.text(), "installing done");
    }

    /// A sequence split across two reads was the first thing to break: the
    /// parser started from scratch on each chunk and left `[0m` in the middle
    /// of a sentence.
    #[test]
    fn an_escape_split_across_two_chunks_is_still_one_escape() {
        let mut s = screen();
        s.write("done \u{1b}[3");
        s.write("2mgreen\u{1b}[0m");
        assert_eq!(s.text(), "done green");
    }

    #[test]
    fn an_osc_carrying_a_path_is_not_text() {
        let mut s = screen();
        s.write("\u{1b}]9;9;\"D:\\work\"\u{7}PS D:\\work> ");
        assert_eq!(s.text(), "PS D:\\work>");
    }

    #[test]
    fn erasing_to_end_of_line_removes_what_was_overwritten() {
        let mut s = screen();
        s.write("a longer line here\r\u{1b}[K");
        s.write("short");
        assert_eq!(s.text(), "short");
    }

    #[test]
    fn output_past_the_bottom_scrolls_into_the_history() {
        let mut s = Screen::new(20, 2, false);
        s.write("one\r\ntwo\r\nthree\r\nfour");
        assert_eq!(s.text(), "one\ntwo\nthree\nfour");
    }

    /// A piped process writes `\n` where a terminal writes `\r\n`. Without the
    /// conversion the whole screen is one long diagonal.
    #[test]
    fn a_process_that_only_writes_newlines_still_gets_lines() {
        let mut s = Screen::new(20, 5, true);
        s.write("first\nsecond\n");
        assert_eq!(s.text(), "first\nsecond");
    }

    #[test]
    fn clearing_the_screen_keeps_the_scrollback_a_person_can_scroll_to() {
        let mut s = Screen::new(20, 2, false);
        s.write("old one\r\nold two\r\nnew\r\n");
        s.write("\u{1b}[2J\u{1b}[1;1H");
        assert!(s.text().contains("old one"), "{}", s.text());
    }

    #[test]
    fn the_history_is_bounded() {
        let mut s = Screen::new(20, 2, false);
        for i in 0..SCROLLBACK_LINES + 200 {
            s.write(&format!("line {i}\r\n"));
        }
        assert!(s.lines.len() <= SCROLLBACK_LINES + 8, "{}", s.lines.len());
        assert!(s
            .text()
            .contains(&format!("line {}", SCROLLBACK_LINES + 199)));
    }
}
