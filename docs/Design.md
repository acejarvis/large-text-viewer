# Large Text Viewer — Architecture (Mermaid)

## Component Map
```mermaid
flowchart TD
    main["main.rs\nentrypoint"] --> app["TextViewerApp\n(app.rs)"]

    subgraph UI [egui]
        menu["Menu/Toolbar/Status/Dialogs"]
        viewport["Central text area\n(show_rows, wrap, highlights, line nos)"]
    end

    subgraph Core [Core]
        fr["FileReader\nmemmap + decoding"]
        li["LineIndexer\nfull/sparse offsets"]
        se["SearchEngine\nchunked literal/regex"]
        watcher["Tail watcher\n(notify)"]
    end

    app --> menu
    app --> viewport
    app --> fr
    app --> li
    app --> se
    app --> watcher

    fr --> li
    fr --> se
    li --> viewport
    se --> viewport

    menu -->|open file| app
    menu -->|encoding select| app
    menu -->|search actions| app
    menu -->|tail toggle| watcher
```

## Search/Data Flow
```mermaid
sequenceDiagram
    participant UI as UI (toolbar/status)
    participant App as TextViewerApp
    participant Reader as FileReader
    participant Indexer as LineIndexer
    participant Search as SearchEngine

    UI->>App: perform_search(query, regex?)
    App->>Search: set_query()
    App->>Search: search(Reader, MAX)
    Search->>Reader: get_bytes(chunk)
    Search-->>App: stored hits + total_results
    App->>UI: status "Found N matches"
    UI->>App: next/prev
    App->>UI: scroll_to_row(target line)

    App->>Indexer: get_line_with_reader(line)
    Indexer->>Reader: get_bytes(span)
    App->>UI: render line with inline highlights
```

## Notes
- File open: UI → open_file → FileReader mmap + LineIndexer index → status update.
- Rendering: visible rows only; LineIndexer supplies byte spans; FileReader decodes; UI draws text + per-match highlights.
- Tail mode: notify watcher reloads file, reindexes, and can auto-scroll to bottom.
- Encoding: selector swaps `selected_encoding` and reopens file.