# PDF Reader - Rust + Slint Desktop Application

A cross-platform PDF reader desktop application built with Rust and Slint UI framework, featuring PDF rendering, page navigation, zoom controls, and thumbnail support.

## Features

- 📖 **PDF Document Loading**: Open and display PDF documents
- 🖼️ **Page Rendering**: High-quality PDF page rendering with zoom support
- 🔍 **Zoom Controls**: Zoom in/out functionality with smooth scaling
- 📑 **Page Navigation**: Navigate through document pages with previous/next buttons
- 🎨 **Thumbnail View**: Optional thumbnail sidebar for quick page navigation
- 💾 **Image Caching**: Efficient caching system for rendered pages and thumbnails
- 🔗 **Link Detection**: Detect and highlight PDF hyperlinks
- 📱 **Responsive UI**: Modern, responsive user interface

## Architecture

The application is structured with the following modules:

- **`pdf/`**: PDF processing and rendering
  - `document.rs`: PDF document management
  - `page.rs`: Individual page handling
  - `renderer.rs`: Page rendering with overlays
- **`cache/`**: Image caching system for performance
- **`state/`**: Application state management
- **`ui/`**: User interface event handling
- **`ui/main.slint`**: Slint UI definition

## Dependencies

- **mupdf**: PDF document processing and rendering
- **slint**: Modern UI framework for desktop applications
- **tokio**: Async runtime for concurrent operations
- **image**: Image processing and manipulation
- **anyhow**: Error handling
- **crossbeam**: Concurrent data structures

## Building and Running

### Prerequisites

1. Install Rust toolchain: https://rustup.rs/
2. Install system dependencies for mupdf:
   ```bash
   # On macOS
   brew install mupdf-tools
   
   # On Ubuntu/Debian
   sudo apt-get install libmupdf-dev
   
   # On Windows
   # Follow mupdf documentation for Windows setup
   ```

### Build

```bash
cd pdf-reader-rust
cargo build --release
```

### Run

```bash
cargo run
```

## Usage

1. **Open PDF**: Click the "Open" button to select a PDF file
2. **Navigate Pages**: Use "Previous" and "Next" buttons to navigate
3. **Zoom**: Use "Zoom In" and "Zoom Out" buttons to adjust magnification
4. **Thumbnails**: Toggle thumbnail sidebar for quick page selection
5. **Page Info**: View current page number and total pages in the status bar

## Development

The project structure follows clean architecture principles:

- **Separation of Concerns**: Each module has a specific responsibility
- **Async Operations**: PDF operations use tokio for non-blocking execution
- **Error Handling**: Comprehensive error handling with anyhow
- **Performance**: Efficient caching and lazy loading
- **Extensibility**: Modular design allows easy feature additions

## Future Enhancements

- [ ] Text selection and copying
- [ ] Search functionality
- [ ] Bookmarks and annotations
- [ ] Print support
- [ ] Multiple document tabs
- [ ] Dark mode support
- [ ] Customizable keyboard shortcuts

## License

This project is part of the KReader ecosystem and follows similar licensing terms.