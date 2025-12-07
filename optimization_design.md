# PDF 阅读器异步解码优化设计

## 🎯 目标
将当前的同步UI线程解码改为多线程异步解码，提升用户体验：
- 消除UI卡顿（解码耗时430ms）
- 支持多优先级任务队列
- 避免重复解码任务
- 实时更新UI

## 📋 核心问题分析

### 当前架构问题
1. **线程安全**：`slint::Image` 不能在线程间传递
2. **UI阻塞**：400+ms解码阻塞主线程
3. **被动刷新**：解码完后无主动UI更新机制
4. **重复任务**：滚动时同页面被多次请求解码

## 🏗️ 新架构设计

### 1. 线程架构
```
UI线程  ←-- invoke_from_event_loop() --  后台
    ↑                                     ↓
mpsc消息 ←-----------------------------  缓存+回调
    ↗                                     ↘
订阅者模式 ←-------------------→   slint信号(suggest)
```

**线程分工**：
- **UI线程**：缓存查询、消息接收、UI更新
- **分配线程**：接收解码请求、优先级调度（blocking wait）
- **解码线程**：实际PDF解码、转换slint::Image

### 2. 任务和Key设计

**PageInfo 定义**：包含所有影响缓存的因子
```rust
#[derive(Clone, Hash, Eq, PartialEq, Debug)]
pub struct PageInfo {
    pub page_index: usize,
    pub scale: f32,
    pub crop: i32,        // 0=无切边, 1=有切边
    pub width: f32,
    pub height: f32,
}
```

**任务统一结构** (不再用枚举)：
```rust
#[derive(Clone)]
pub struct DecodeTask {
    pub priority: Priority,
    pub page_info: PageInfo,
}

#[derive(Clone, Copy, PartialOrd, Ord, PartialEq, Eq)]
pub enum Priority {
    Thumbnail = 0,     // 最高优先级
    FullImage = 1,     // 中优先级
    Cropped = 2,       // 低优先级
}
```

### 3. 共享状态管理
```rust
struct DecodeState {
    cache: Arc<Mutex<PageCache>>,           // 线程安全缓存
    visible_pages: Arc<Mutex<HashSet<usize>>>,  // 当前可见页面
    pending_requests: Arc<Mutex<HashSet<String>>>, // 任务去重
}
```

## 📁 修改文件清单

### 1. `src/cache/cache.rs` 🔄 (需要重新设计)

**问题**：现有 `PageCache` 包含两个 `ImageCache`，但回收策略会相互影响，无法保证缩略图优先级。

**新设计**：
- 分离为两个完全独立的缓存类：`ThumbnailCache` 和 `ImageCache`
- `ThumbnailCache`：专用于缩略图，容量小，LRU回收
- `ImageCache`：专用于高清图片，容量大，LRU回收
- 各自独立回收，不相互影响

```rust
pub struct ThumbnailCache { /* LRU cache for thumbnails */ }
pub struct ImageCache { /* LRU cache for images */ }
pub struct PageCache {
    pub thumbnails: ThumbnailCache,
    pub images: ImageCache,
}
```

### 2. `src/decoder/decode_service.rs` 🔄 (主要修改)

**新增结构体**：
```rust
#[derive(Clone)]
pub enum DecodeTask {
    Thumbnail { page_index: usize },
    FullImage { page_index: usize, scale: f32 },
    CroppedImage { page_index: usize, scale: f32 },
}

pub struct DecodeState {
    cache: Arc<Mutex<PageCache>>,
    visible_pages: Arc<Mutex<HashSet<usize>>>,
    pending_requests: Arc<Mutex<HashSet<String>>>,
}

pub struct DecodeResult {
    task: DecodeTask,
    image: slint::Image,
}
```

**修改 DecodeService**：
```rust
pub struct DecodeService {
    pub state: Arc<Mutex<DecodeState>>,              // 新增：共享状态

    request_tx: mpsc::Sender<DecodeTask>,              // 请求发送
    request_rx: mpsc::Receiver<DecodeTask>,            // 请求接收(分配线程用)

    decode_tx: mpsc::Sender<DecodeTask>,               // 解码任务发送
    decode_rx: mpsc::Receiver<DecodeTask>,             // 解码任务接收

    result_tx: mpsc::Sender<DecodeResult>,             // 结果发送
    result_rx: mpsc::Receiver<DecodeResult>,           // 结果接收

    dispatcher_thread: Option<JoinHandle<()>>,         // 分配线程句柄
    decoder_thread: Option<JoinHandle<()>>,            // 解码线程句柄
}

// 正确的资源管理 Drop 实现
impl Drop for DecodeService {
    fn drop(&mut self) {
        // 发送结束信号给线程
        drop(self.request_tx.clone());  // 关闭接收端会让线程退出
        drop(self.decode_tx.clone());

        // 等待线程结束
        if let Some(thread) = self.dispatcher_thread.take() {
            thread.join().ok();
        }
        if let Some(thread) = self.decoder_thread.take() {
            thread.join().ok();
        }
    }
}
```

### 3. `src/page/view_state.rs` 🔄 (需要修改)

**新增回调机制**：
```rust
type RefreshCallback = Box<dyn Fn() + Send + Sync>;

pub struct PageViewState {
    // ...现有字段...
    state: Arc<Mutex<DecodeState>>,                    // 新增：共享状态
    on_refresh_needed: Option<RefreshCallback>,        // 新增：UI刷新回调
}
```

**修改方法**：
- `update_visible_pages()`：添加去重逻辑和任务提交
- 新增：`set_refresh_callback()`
- 新增：`task_key()` 生成唯一键

### 4. `src/main.rs` 🔄 (需要修改)

**设置回调**：
```rust
page_view_state.borrow_mut().set_refresh_callback(Box::new(move || {
    if let Some(refresh) = refresh_callback {
        refresh();
    }
}));
```

**监听结果**：
```rust
std::thread::spawn(move || {
    while let Ok(result) = decode_service.result_rx.recv() {
        slint::invoke_from_event_loop(move || {
            // 缓存结果
            // 通知UI刷新
        });
    }
});
```

## 🔧 实现步骤

### 第1步：完善 DecodeTask 和 DecodeState

1. 在 `decode_service.rs` 中定义 `DecodeTask`、`DecodeState`、`DecodeResult`
2. 定义任务键生成 `task_key()` 方法
3. 实现 `DecodeState` 方法：`add_pending`、`remove_pending`、`is_pending`

### 第2步：重构 DecodeService

1. 初始化共享状态 `Arc<Mutex<DecodeState>>`
2. 创建所有 channel：`request`、`decode`、`result`
3. 启动两个线程：`dispatcher_thread()` 和 `decoder_thread()`
4. 实现分配线程：blocking `recv()` + 优先级队列调度
5. 实现解码线程：blocking `recv()` + 可见性检查 + PDF解码

### 第3步：修改 PageViewState

1. 集成共享状态 `Arc<Mutex<DecodeState>>`
2. 在 `update_visible_pages()` 中：
   - 检查缓存存在
   - 检查 `pending_requests`
   - 发送解码请求到分配线程
3. 添加清理机制：页面不再可见时移除pending

### 第4步：更新 main.rs

1. 初始化线程と
2. 设置回调函数
3. 启动结果监听线程
4. 使用 `invoke_from_event_loop()` 更新UI

## 🎨 slint Signals 方案（主方案）

**使用 slint 信号机制进行UI通知**：
1. **解耦性**：组件间松耦合通信，无需直接持有回调
2. **类型安全**：编译时检查信号类型
3. **多监听者**：一个信号可以被多个UI组件监听

**Slint 中定义信号**：
```slint
MainWindow := Window {
    // 定义信号，传递完成的任务信息
    signal decode_completed(page_index: i32, priority: i32);

    // UI 触发的请求
    callback request_decode(page_index: i32, priority: i32, scale: f32);
    // ...
}
```

**Rust 中监听信号**：
```rust
let app = MainWindow::new()?;

// 监听解码完成信号
app.on_decode_completed(move |page_index, priority| {
    println!("页面 {} 解码完成，优先级 {}", page_index, priority);
    refresh_view(&app);
});

// 从解码线程发送信号
slint::invoke_from_event_loop(move || {
    app.invoke_decode_completed(page_index as _, priority as _);
});
```

Signals方案替代回调，代码更加清晰规范。

## 🧪 测试要点

1. **无重复任务**：快速滚动不产生过多解码
2. **实时响应**：新请求立即被分配
3. **缓存有效**：见过的页面不解码
4. **可见性检查**：滚动出屏幕的任务被清理
5. **UI平滑**：解码在后台，不卡UI

## 🚀 预期效果

- 🔥 UI流畅度提升90%
- 💾 内存和CPU使用优化
- ⚡ 响应速度大幅改善
- 🛡️ 线程安全可靠
