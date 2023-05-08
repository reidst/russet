#![no_std]
#![feature(prelude_2024)]

use pc_keyboard::{DecodedKey, KeyCode};
use pluggable_interrupt_os::vga_buffer::{BUFFER_WIDTH, BUFFER_HEIGHT, plot, ColorCode, Color, plot_str, is_drawable, plot_num};
use csci320_vsfs::FileSystem;
use simple_interp::{Interpreter, InterpreterOutput, i64_into_buffer};
use gc_headers::GarbageCollectingHeap;
// use gc_heap::CopyingHeap;

// Get rid of some spurious VSCode errors
use core::option::Option;
use core::option::Option::None;
use core::prelude::rust_2024::derive;
use core::clone::Clone;
use core::cmp::{PartialEq,Eq};
use core::marker::Copy;
use core::str;

const FIRST_BORDER_ROW: usize = 1;
const LAST_BORDER_ROW: usize = BUFFER_HEIGHT - 1;
const TASK_MANAGER_WIDTH: usize = 10;
const TASK_MANAGER_BYTES: usize = BUFFER_HEIGHT * TASK_MANAGER_WIDTH;
const WINDOWS_WIDTH: usize = BUFFER_WIDTH - TASK_MANAGER_WIDTH;
const WINDOW_WIDTH: usize = (WINDOWS_WIDTH - 3) / 2;
const WINDOW_HEIGHT: usize = (LAST_BORDER_ROW - FIRST_BORDER_ROW - 2) / 2;
const MID_WIDTH: usize = WINDOWS_WIDTH / 2;
const MID_HEIGHT: usize = BUFFER_HEIGHT / 2;
const NUM_WINDOWS: usize = 4;
const WINDOW_LABEL_COL_OFFSET: usize = WINDOW_WIDTH - 3;
const FILENAME_LABEL_COL_OFFSET: usize = 2;

const FILENAME_PROMPT: &str = "F5 - Filename: ";
const EDIT_MODE_HEADER: &str = "(F6)";

const MAX_OPEN: usize = 16;
const BLOCK_SIZE: usize = 256;
const NUM_BLOCKS: usize = 255;
const MAX_FILE_BLOCKS: usize = 64;
const MAX_FILE_BYTES: usize = MAX_FILE_BLOCKS * BLOCK_SIZE;
const MAX_FILES_STORED: usize = 30;
const MAX_FILENAME_BYTES: usize = 10;

const PRACTICAL_FILE_BUFFER_SIZE: usize = MAX_FILE_BYTES - 1;  // i made an oopsie in vsfs

const MAX_TOKENS: usize = 500;
const MAX_LITERAL_CHARS: usize = 30;
const STACK_DEPTH: usize = 50;
const MAX_LOCAL_VARS: usize = 20;
const HEAP_SIZE: usize = 1024;
const MAX_HEAP_BLOCKS: usize = HEAP_SIZE;

// Data type for a file system object:
// FileSystem<MAX_OPEN, BLOCK_SIZE, NUM_BLOCKS, MAX_FILE_BLOCKS, MAX_FILE_BYTES, MAX_FILES_STORED, MAX_FILENAME_BYTES>

// Data type for an interpreter object:
// Interpreter<MAX_TOKENS, MAX_LITERAL_CHARS, STACK_DEPTH, MAX_LOCAL_VARS, WINDOW_WIDTH, CopyingHeap<HEAP_SIZE, MAX_HEAP_BLOCKS>>


#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum KWindows { F1, F2, F3, F4 }

impl KWindows {
    fn col(&self) -> usize {
        match self {
            KWindows::F1 => 0,
            KWindows::F2 => MID_WIDTH - 1,
            KWindows::F3 => 0,
            KWindows::F4 => MID_WIDTH - 1,
        }
    }
    fn row(&self) -> usize {
        match self {
            KWindows::F1 => FIRST_BORDER_ROW,
            KWindows::F2 => FIRST_BORDER_ROW,
            KWindows::F3 => MID_HEIGHT,
            KWindows::F4 => MID_HEIGHT,
        }
    }
    fn name(&self) -> &str {
        match self {
            KWindows::F1 => "F1",
            KWindows::F2 => "F2",
            KWindows::F3 => "F3",
            KWindows::F4 => "F4",
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct DirectoryState {
    cursor: usize,
}

impl DirectoryState {
    fn move_cursor(&mut self, delta: isize, file_count: usize) {
        let new_pos = self.cursor as isize + delta;
        if new_pos >= 0 && new_pos < file_count as isize {
            self.cursor = new_pos as usize;
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct EditingState {
    filename: [u8; MAX_FILENAME_BYTES],
    buffer: [u8; PRACTICAL_FILE_BUFFER_SIZE],
    len: usize,
    cursor: usize,
    scroll: usize,
    directory_index: usize,
}

impl EditingState {
    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.buffer[self.cursor] = 0;
            self.len -= 1;
        }
    }

    fn type_char(&mut self, c: char) {
        if self.cursor < PRACTICAL_FILE_BUFFER_SIZE {
            self.buffer[self.cursor] = c as u8;
            self.cursor += 1;
            self.len += 1;
        }
    }

    fn line_count(&self, line_width: usize) -> usize {
        let mut count = 1;
        let mut cursor = 0;
        let mut len = 0;
        loop {
            let &this_byte = match self.buffer.get(cursor) {
                Some(byte) if byte == &0 => break,
                Some(byte) => byte,
                None => break,
            };
            if this_byte == '\n' as u8 {
                count += 1;
                cursor += 2;
                len = 0;
            } else if len == line_width {
                count += 1;
                cursor += 1;
                len = 0;
            } else {
                cursor += 1;
                len += 1;
            }
        }
        count
    }

    fn read_line(&self, line: usize) -> Option<[u8; WINDOW_WIDTH]> {
        let mut line_buf = [' ' as u8; WINDOW_WIDTH];
        let mut current_line = 0;
        let mut line_start = 0;
        let mut line_len = 0;
        loop {
            if current_line > line { break }
            let &this_byte = match self.buffer.get(line_start + line_len) {
                Some(byte) => byte,
                None => break,
            };
            if this_byte == '\n' as u8 {
                current_line += 1;
                line_start += line_len + 1;
                line_len = 0;
            } else if line_len == WINDOW_WIDTH {
                current_line += 1;
                line_start += line_len;
                line_len = 0;
            } else {
                if current_line == line {
                    line_buf[line_len] = this_byte;
                }
                line_len += 1;
            }
        }

        if current_line > line {
            Some(line_buf)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct RunningState {
    interpreter: Interpreter<
        MAX_TOKENS,
        MAX_LITERAL_CHARS,
        STACK_DEPTH,
        MAX_LOCAL_VARS,
        WINDOW_WIDTH,
        DummyHeap<HEAP_SIZE, MAX_HEAP_BLOCKS>,
    >,
}

// dummy struct, allows interpreter to compile
#[derive(Clone, Copy, Debug)]
struct DummyHeap<const HEAP_SIZE: usize, const MAX_HEAP_BLOCKS: usize>;
impl GarbageCollectingHeap for DummyHeap<HEAP_SIZE, MAX_HEAP_BLOCKS> {
    fn new() -> Self {todo!("dummy heap")}
    fn load(&self, p: gc_headers::Pointer) -> gc_headers::HeapResult<u64> {todo!("dummy heap")}
    fn store(&mut self, p: gc_headers::Pointer, value: u64) -> gc_headers::HeapResult<()> {todo!("dummy heap")}
    fn malloc<T: gc_headers::Tracer>(&mut self, num_words: usize, tracer: &T) -> gc_headers::HeapResult<gc_headers::Pointer> {todo!("dummy heap")}
}

#[derive(Clone, Copy, Debug)]
enum KWindowMode {
    Directory(DirectoryState),
    Editing(EditingState),
    Running(RunningState),
}

impl KWindowMode {
    fn directory(cursor: usize) -> Self {
        Self::Directory(DirectoryState { cursor })
    }

    fn editing(
        filename: [u8; MAX_FILENAME_BYTES],
        buffer: [u8; PRACTICAL_FILE_BUFFER_SIZE],
        len: usize,
        directory_index: usize,
    ) -> Self {
        let mut state = EditingState {
            filename,
            buffer,
            len,
            cursor: len,
            scroll: 0,
            directory_index,
        };
        state.scroll = state.line_count(WINDOW_WIDTH).saturating_sub(WINDOW_HEIGHT);
        Self::Editing(state)
    }

    fn running(program: &str) -> Self {
        Self::Running(RunningState {
            interpreter: Interpreter::new(program)
        })
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum KSelection { Window(KWindows), Filebar }

struct TypingBuffer<const MAX_LENGTH: usize> {
    buffer: [u8; MAX_LENGTH],
    cursor: usize,
}

impl TypingBuffer<MAX_FILENAME_BYTES> {
    fn type_char(&mut self, c: char) {
        if self.cursor < MAX_FILENAME_BYTES {
            self.buffer[self.cursor] = c as u8;
            self.cursor += 1;
        }
    }

    fn backspace(&mut self) {
        if self.cursor > 0 {
            self.buffer[self.cursor - 1] = 0;
            self.cursor -= 1;
        }
    }

    fn clear(&mut self) {
        self.buffer = [0; MAX_FILENAME_BYTES];
        self.cursor = 0;
    }

    fn draw(&self, col: usize, row: usize, color: ColorCode) {
        for i in 0..MAX_FILENAME_BYTES {
            let char_to_plot = if i < self.cursor { self.buffer[i] as char } else { ' ' };
            plot(char_to_plot, col + i, row, color);
        }
    }

    fn get_bytes(&mut self) -> (usize, [u8; MAX_FILENAME_BYTES]) {
        (self.cursor, self.buffer.clone())
    }
}

pub struct Kernel {
    selected: KSelection,
    filebar_buffer: TypingBuffer<MAX_FILENAME_BYTES>,
    window_modes: [KWindowMode; 4],
    fs: FileSystem<
        MAX_OPEN, 
        BLOCK_SIZE, 
        NUM_BLOCKS, 
        MAX_FILE_BLOCKS, 
        MAX_FILE_BYTES, 
        MAX_FILES_STORED, 
        MAX_FILENAME_BYTES
    >,
}

const HELLO: &str = r#"print("Hello, world!")"#;

const NUMS: &str = r#"print(1)
print(257)"#;

const ADD_ONE: &str = r#"x := input("Enter a number")
x := (x + 1)
print(x)"#;

const COUNTDOWN: &str = r#"count := input("count")
while (count > 0) {
    count := (count - 1)
}
print("done")
print(count)"#;

const AVERAGE: &str = r#"sum := 0
count := 0
averaging := true
while averaging {
    num := input("Enter a number:")
    if (num == "quit") {
        averaging := false
    } else {
        sum := (sum + num)
        count := (count + 1)
    }
}
print((sum / count))"#;

const PI: &str = r#"sum := 0
i := 0
neg := false
terms := input("Num terms:")
while (i < terms) {
    term := (1.0 / ((2.0 * i) + 1.0))
    if neg {
        term := -term
    }
    sum := (sum + term)
    neg := not neg
    i := (i + 1)
}
print((4 * sum))"#;

// Seed the disk with some programs.
fn initial_files(disk: &mut FileSystem<MAX_OPEN, BLOCK_SIZE, NUM_BLOCKS, MAX_FILE_BLOCKS, MAX_FILE_BYTES, MAX_FILES_STORED, MAX_FILENAME_BYTES>) {
    for (filename, contents) in [
        ("hello", HELLO),
        ("nums", NUMS),
        ("add_one", ADD_ONE),
        ("countdown", COUNTDOWN),
        ("average", AVERAGE),
        ("pi", PI),
    ] {
        let fd = disk.open_create(filename).unwrap();
        disk.write(fd, contents.as_bytes()).unwrap();
        disk.close(fd);
    }
}

impl Kernel {
    pub fn new() -> Self {
        let mut fs: FileSystem<
            MAX_OPEN, 
            BLOCK_SIZE, 
            NUM_BLOCKS, 
            MAX_FILE_BLOCKS, 
            MAX_FILE_BYTES, 
            MAX_FILES_STORED, 
            MAX_FILENAME_BYTES
        > = FileSystem::new(ramdisk::RamDisk::new());
        initial_files(&mut fs);
        let filebar_buffer = TypingBuffer {
            buffer: [0u8; MAX_FILENAME_BYTES],
            cursor: 0,
        };
        
        Self {
            selected: KSelection::Window(KWindows::F1),
            filebar_buffer,
            window_modes: [KWindowMode::directory(0); NUM_WINDOWS],
            fs
        }
    }

    pub fn key(&mut self, key: DecodedKey) {
        match key {
            DecodedKey::RawKey(code) => self.handle_raw(code),
            DecodedKey::Unicode(c) => self.handle_unicode(c)
        }
        self.draw();
    }

    fn handle_raw(&mut self, key: KeyCode) {
        match key {
            KeyCode::F1 => self.selected = KSelection::Window(KWindows::F1),
            KeyCode::F2 => self.selected = KSelection::Window(KWindows::F2),
            KeyCode::F3 => self.selected = KSelection::Window(KWindows::F3),
            KeyCode::F4 => self.selected = KSelection::Window(KWindows::F4),
            KeyCode::F5 => self.selected = KSelection::Filebar,
            KeyCode::F6 => {
                if let KSelection::Window(window) = self.selected {
                    self.switch_to_directory_mode(window);
                }
            },
            KeyCode::F7 => self.scroll_edit_text(-1),
            KeyCode::F8 => self.scroll_edit_text(1),
            KeyCode::ArrowUp    => self.move_dir_cursor(-3),
            KeyCode::ArrowDown  => self.move_dir_cursor(3),
            KeyCode::ArrowLeft  => self.move_dir_cursor(-1),
            KeyCode::ArrowRight => self.move_dir_cursor(1),
            _ => {}
        }
    }

    fn handle_unicode(&mut self, key: char) {
        match self.selected {
            KSelection::Filebar => {
                match key {
                    '\u{8}' => self.filebar_buffer.backspace(),
                    '\n' => self.try_create_file(),
                    other if is_drawable(other) => self.filebar_buffer.type_char(other),
                    _ => {},
                }
            },
            KSelection::Window(window) => {
                match self.get_window_mode(window) {
                    KWindowMode::Directory(_) => {
                        match key {
                            'e' => self.switch_to_edit_mode(window),
                            'r' => self.switch_to_run_mode(window),
                            _ => {},
                        }
                    },
                    KWindowMode::Editing(mut edit_state) => {
                        match key {
                            '\n' => edit_state.type_char('\n'),
                            key if is_drawable(key) => edit_state.type_char(key),
                            '\u{8}' => edit_state.backspace(),
                            _ => {},
                        }
                        self.set_window_mode(window, KWindowMode::Editing(edit_state));
                    },
                    KWindowMode::Running(_) => {
                        todo!("handle unicode for a running window")
                    },
                }
            },
        }
    }

    pub fn draw(&mut self) {
        plot_str(FILENAME_PROMPT, 0, 0, text_color());
        self.filebar_buffer.draw(FILENAME_PROMPT.len(), 0, text_color());
        for window in [KWindows::F1, KWindows::F2, KWindows::F3, KWindows::F4] {
            self.draw_window(window);
        }
        if let KSelection::Window(window) = self.selected {
            self.draw_window(window)
        }
        for window in [KWindows::F1, KWindows::F2, KWindows::F3, KWindows::F4] {
            plot_str(
                window.name(),
                window.col() + WINDOW_LABEL_COL_OFFSET,
                window.row(),
                text_color(),
            );
        }
    }

    pub fn draw_proc_status(&mut self) {
        // todo!("Draw processor status");
    }

    pub fn run_one_instruction(&mut self) {
        // todo!("Run an instruction in a process");
    }

    fn draw_window(&mut self, window: KWindows) {
        self.clear_window(window);
        self.draw_window_border(window);
        let col = window.col();
        let row = window.row();
        match self.get_window_mode(window) {
            KWindowMode::Directory(dir_state) => {
                let (file_count, filenames) = self.fs.list_directory().unwrap();
                let mut file_col_offset = 1;
                let mut file_row_offset = 1;
                for file in 0..file_count {
                    let filename_bytes = filenames[file];
                    for byte in filename_bytes {
                        let color = if file == dir_state.cursor { highlight_color() } else { text_color() };
                        plot(byte as char, col + file_col_offset, row + file_row_offset, color);
                        file_col_offset += 1;
                    }
                    if file_col_offset > 3 * MAX_FILENAME_BYTES {
                        file_col_offset = 1;
                        file_row_offset += 1;
                    }
                }
            },
            KWindowMode::Editing(edit_state) => {
                plot_str(EDIT_MODE_HEADER, col + FILENAME_LABEL_COL_OFFSET, row, text_color());
                for i in 0..edit_state.filename.len() {
                    if edit_state.filename[i] == 0 { continue }
                    plot(
                        edit_state.filename[i] as char,
                        col + i + EDIT_MODE_HEADER.len() + FILENAME_LABEL_COL_OFFSET,
                        row,
                        text_color()
                    );
                }
                for line in 0..WINDOW_HEIGHT {
                    if let Some(line_bytes) = edit_state.read_line(edit_state.scroll + line) {
                        let line_str = str::from_utf8(&line_bytes).unwrap();
                        plot_str(line_str, col + 1, row + 1 + line, text_color());
                    } else {
                        continue
                    }
                }
            },
            KWindowMode::Running(_) => {
                todo!("draw a running window")
            },
        }
    }

    fn draw_window_border(&mut self, window: KWindows) {
        let col = window.col();
        let row = window.row();
        let border = if let KSelection::Window(selected_win) = self.selected {
            if selected_win == window {'*'} else {'.'}
        } else {'.'};
        for col_offset in 0..WINDOW_WIDTH+2 {
            plot(border, col + col_offset, row, text_color());
            plot(border, col + col_offset, row + WINDOW_HEIGHT+1, text_color());
        }
        for row_offset in 0..WINDOW_HEIGHT+2 {
            plot(border, col, row + row_offset, text_color());
            plot(border, col + WINDOW_WIDTH+1, row + row_offset, text_color());
        }
    }

    fn clear_window(&mut self, window: KWindows) {
        let col = window.col();
        let row = window.row();
        for col_offset in 1..WINDOW_WIDTH+1 {
            for row_offset in 1..WINDOW_HEIGHT+1 {
                plot(' ', col + col_offset, row + row_offset, text_color());
            }
        }
    }

    fn try_create_file(&mut self) {
        let (name_len, name_bytes) = self.filebar_buffer.get_bytes();
        self.filebar_buffer.clear();
        if let Ok(str) = str::from_utf8(&name_bytes[0..name_len]) {
            let new_file = self.fs.open_create(str).unwrap();
            self.fs.close(new_file).unwrap();
        }
    }

    fn get_window_mode(&self, window: KWindows) -> KWindowMode {
        match window {
            KWindows::F1 => self.window_modes[0],
            KWindows::F2 => self.window_modes[1],
            KWindows::F3 => self.window_modes[2],
            KWindows::F4 => self.window_modes[3],
        }
    }

    fn set_window_mode(&mut self, window: KWindows, mode: KWindowMode) {
        let index = match window {
            KWindows::F1 => 0,
            KWindows::F2 => 1,
            KWindows::F3 => 2,
            KWindows::F4 => 3,
        };
        self.window_modes[index] = mode;
    }

    fn move_dir_cursor(&mut self, delta: isize) {
        if let KSelection::Window(window) = self.selected {
            if let KWindowMode::Directory(mut dir_state) = self.get_window_mode(window) {
                let (file_count, _) = self.fs.list_directory().unwrap();
                dir_state.move_cursor(delta, file_count);
                self.set_window_mode(window, KWindowMode::Directory(dir_state));
            }
        }
    }

    fn scroll_edit_text(&mut self, delta: isize) {
        if let KSelection::Window(window) = self.selected {
            if let KWindowMode::Editing(mut edit_state) = self.get_window_mode(window) {
                edit_state.scroll = edit_state.scroll.saturating_add_signed(delta);
                let line_count = edit_state.line_count(WINDOW_WIDTH);
                if edit_state.scroll >= line_count {
                    edit_state.scroll = line_count - 1;
                }
                self.set_window_mode(window, KWindowMode::Editing(edit_state));
            }
        }
    }

    fn switch_to_edit_mode(&mut self, window: KWindows) {
        if let KWindowMode::Directory(dir_state) = self.get_window_mode(window) {
            let chosen_file = dir_state.cursor;
            let (file_count, directory) = self.fs.list_directory().unwrap();
            assert!(chosen_file < file_count);
            let filename_str = str::from_utf8(&directory[chosen_file]).unwrap();
            let file = self.fs.open_read(filename_str).unwrap();
            let mut buffer = [0u8; PRACTICAL_FILE_BUFFER_SIZE];
            let filesize = self.fs.read(file, &mut buffer).unwrap();
            self.fs.close(file);
            self.set_window_mode(
                window,
                KWindowMode::editing(directory[chosen_file], buffer, filesize, chosen_file),
            );
        }
    }

    fn switch_to_directory_mode(&mut self, window: KWindows) {
        if let KWindowMode::Editing(edit_state) = self.get_window_mode(window) {
            let filename_str = str::from_utf8(&edit_state.filename).unwrap();
            let file = self.fs.open_create(filename_str).unwrap();
            self.fs.write(file, &edit_state.buffer[0..edit_state.len]).unwrap();
            self.fs.close(file).unwrap();
            self.set_window_mode(
                window,
                KWindowMode::directory(edit_state.directory_index),
            );
        }
    }

    fn switch_to_run_mode(&mut self, window:KWindows) {
        if let KWindowMode::Directory(dir_state) = self.get_window_mode(window) {
            let chosen_file = dir_state.cursor;
            let (file_count, directory) = self.fs.list_directory().unwrap();
            assert!(chosen_file < file_count);
            let filename_str = str::from_utf8(&directory[chosen_file]).unwrap();
            let file = self.fs.open_read(filename_str).unwrap();
            let mut buffer = [0u8; PRACTICAL_FILE_BUFFER_SIZE];
            let filesize = self.fs.read(file, &mut buffer).unwrap();
            self.fs.close(file);
            let program = str::from_utf8(&buffer[..filesize]).unwrap();
            self.set_window_mode(
                window,
                KWindowMode::running(program),
            );
        }
    }
}

fn text_color() -> ColorCode {
    ColorCode::new(Color::White, Color::Black)
}

fn highlight_color() -> ColorCode {
    ColorCode::new(Color::Black, Color::White)
}
