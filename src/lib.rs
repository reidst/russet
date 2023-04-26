#![no_std]
#![feature(prelude_2024)]

use pc_keyboard::{DecodedKey, KeyCode};
use pluggable_interrupt_os::vga_buffer::{BUFFER_WIDTH, BUFFER_HEIGHT, plot, ColorCode, Color, plot_str, is_drawable, plot_num};
use ramdisk::RamDisk;
use csci320_vsfs::FileSystem;
use simple_interp::{Interpreter, InterpreterOutput, i64_into_buffer};
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

const FILENAME_PROMPT: &str = "F5 - Filename: ";

const MAX_OPEN: usize = 16;
const BLOCK_SIZE: usize = 256;
const NUM_BLOCKS: usize = 255;
const MAX_FILE_BLOCKS: usize = 64;
const MAX_FILE_BYTES: usize = MAX_FILE_BLOCKS * BLOCK_SIZE;
const MAX_FILES_STORED: usize = 30;
const MAX_FILENAME_BYTES: usize = 10;

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

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
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

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
struct EditingState;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
enum KWindowMode {
    Directory(DirectoryState),
    Editing(EditingState),
}

impl KWindowMode {
    fn directory(cursor: usize) -> Self {
        Self::Directory(DirectoryState { cursor })
    }

    fn editing() -> Self {
        Self::Editing(EditingState)
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
            window_modes: [KWindowMode::directory(0); 4],
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
                    self.set_window_mode(window, KWindowMode::directory(0));
                }
            },
            KeyCode::ArrowUp    => self.move_cursor(-3),
            KeyCode::ArrowDown  => self.move_cursor(3),
            KeyCode::ArrowLeft  => self.move_cursor(-1),
            KeyCode::ArrowRight => self.move_cursor(1),
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
                match key {
                    'e' => self.set_window_mode(window, KWindowMode::editing()),
                    _ => {},
                }
            },
        }
    }

    pub fn draw(&mut self) {
        plot_str(FILENAME_PROMPT, 0, 0, text_color());
        self.filebar_buffer.draw(FILENAME_PROMPT.len(), 0, text_color());
        for win in [KWindows::F1, KWindows::F2, KWindows::F3, KWindows::F4] {
            self.draw_window(win);
            plot_str(win.name(), win.col() + WINDOW_LABEL_COL_OFFSET, win.row(), text_color());
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
            KWindowMode::Editing(_) => {
                plot('E', col + 9, row + 5, text_color());
                plot('D', col + 12, row + 4, text_color());
                plot('I', col + 15, row + 3, text_color());
                plot('T', col + 18, row + 2, text_color());

                plot('M', col + 13, row + 8, text_color());
                plot('O', col + 16, row + 7, text_color());
                plot('D', col + 19, row + 6, text_color());
                plot('E', col + 22, row + 5, text_color());
            },
        }
    }

    fn draw_window_border(&mut self, window: KWindows) {
        let col = window.col();
        let row = window.row();
        let border = if let KSelection::Window(selected_win) = self.selected {
            if selected_win == window {'*'} else {'.'}
        } else {'.'};
        for col_offset in 0..=WINDOW_WIDTH {
            plot(border, col + col_offset, row, text_color());
            plot(border, col + col_offset, row + WINDOW_HEIGHT, text_color());
        }
        for row_offset in 0..=WINDOW_HEIGHT {
            plot(border, col, row + row_offset, text_color());
            plot(border, col + WINDOW_WIDTH, row + row_offset, text_color());
        }
    }

    fn clear_window(&mut self, window: KWindows) {
        let col = window.col();
        let row = window.row();
        for col_offset in 1..WINDOW_WIDTH {
            for row_offset in 1..WINDOW_HEIGHT {
                plot(' ', col + col_offset, row + row_offset, text_color());
            }
        }
    }

    fn try_create_file(&mut self) {
        let (name_len, name_bytes) = self.filebar_buffer.get_bytes();
        self.filebar_buffer.clear();
        if let Ok(str) = str::from_utf8(&name_bytes[0..name_len]) {
            self.fs.open_create(str);
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

    fn move_cursor(&mut self, delta: isize) {
        if let KSelection::Window(window) = self.selected {
            if let KWindowMode::Directory(mut dir_state) = self.get_window_mode(window) {
                let (file_count, _) = self.fs.list_directory().unwrap();
                dir_state.move_cursor(delta, file_count);
                self.set_window_mode(window, KWindowMode::Directory(dir_state));
            }
        }
    }
}

fn text_color() -> ColorCode {
    ColorCode::new(Color::White, Color::Black)
}

fn highlight_color() -> ColorCode {
    ColorCode::new(Color::Black, Color::White)
}
