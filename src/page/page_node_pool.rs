use super::PageNode;
use crate::decoder::Rect;

/// PageNode 对象池，避免高频创建/销毁导致的内存抖动
/// 设计文档：大小限制 32 个，使用 acquire/release 管理
pub struct PageNodePool {
    pool: std::collections::VecDeque<PageNode>,
    max_size: usize,
}

impl PageNodePool {
    const DEFAULT_MAX_SIZE: usize = 32;

    pub fn new() -> Self {
        Self {
            pool: std::collections::VecDeque::with_capacity(Self::DEFAULT_MAX_SIZE),
            max_size: Self::DEFAULT_MAX_SIZE,
        }
    }

    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            pool: std::collections::VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    pub fn acquire(&mut self, page_index: usize, bounds: Rect) -> PageNode {
        if let Some(mut node) = self.pool.pop_front() {
            node.recycle();
            node.update(page_index, bounds);
            node
        } else {
            PageNode::new(page_index, bounds)
        }
    }

    pub fn release(&mut self, mut node: PageNode) {
        node.recycle();
        if self.pool.len() < self.max_size {
            self.pool.push_back(node);
        }
    }

    pub fn clear(&mut self) {
        self.pool.clear();
    }

    pub fn size(&self) -> usize {
        self.pool.len()
    }
}

impl Default for PageNodePool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_acquire_release() {
        let mut pool = PageNodePool::with_max_size(2);

        let node1 = pool.acquire(0, Rect::new(0.0, 0.0, 0.5, 0.5));
        assert_eq!(node1.page_index, 0);
        
        pool.release(node1);
        assert_eq!(pool.size(), 1);
        
        let node2 = pool.acquire(1, Rect::new(0.5, 0.0, 1.0, 0.5));
        assert_eq!(pool.size(), 0);
        assert_eq!(node2.page_index, 1);
    }

    #[test]
    fn test_pool_overflow() {
        let mut pool = PageNodePool::with_max_size(2);
        
        let node1 = pool.acquire(0, Rect::new(0.0, 0.0, 0.5, 0.5));
        let node2 = pool.acquire(1, Rect::new(0.0, 0.5, 0.5, 1.0));
        let node3 = pool.acquire(2, Rect::new(0.5, 0.0, 1.0, 0.5));
        
        pool.release(node1);
        pool.release(node2);
        assert_eq!(pool.size(), 2);
        
        // 池已满，node3 被丢弃
        pool.release(node3);
        assert_eq!(pool.size(), 2);
    }
}
