use crate::dao::RecentDao;
use crate::entity::Recent;

/// 数据库CRUD操作测试函数
/// 演示所有数据库操作：创建、读取、更新、删除
pub async fn crud() {
    println!("=== 开始数据库CRUD测试 ===");

    // 1. 创建（Create） - 插入测试记录
    println!("\n1. 创建记录");
    let test_recent = Recent::encode(
        "/path/to/test.pdf".to_string(),
        0, 100, 1, 1, 0, 1.0, 0, 0,
        "test.pdf".to_string(),
        "pdf".to_string(),
        1024,
        1, 0, 0, 1
    );

    match RecentDao::insert_sync(test_recent) {
        Ok(record) => {
            println!("✓ 成功插入记录，ID: {}", record.id);
            let _inserted_id = record.id;

            // 2. 读取（Read） - 根据ID查找
            println!("\n2. 通过ID查找记录");
            match RecentDao::find_by_id_sync(record.id) {
                Ok(Some(found)) => println!("✓ 找到记录: {}", found.book_path),
                Ok(None) => println!("✗ 记录不存在"),
                Err(e) => println!("✗ 查询失败: {}", e),
            }

            // 查找所有记录
            println!("\n3. 查询所有记录");
            match RecentDao::find_all_sync() {
                Ok(records) => println!("✓ 数据库中共有 {} 条记录", records.len()),
                Err(e) => println!("✗ 查询失败: {}", e),
            }

            // 4. 更新（Update） - 修改刚刚插入的记录
            println!("\n4. 更新记录");
            let mut update_recent = Recent::new("/path/to/updated.pdf".to_string());
            update_recent.page = 50;
            update_recent.read_times = 2;

            match RecentDao::update_sync(record.id, &update_recent) {
                Ok(()) => println!("✓ 成功更新记录"),
                Err(e) => println!("✗ 更新失败: {}", e),
            }

            // 验证更新结果
            match RecentDao::find_by_id_sync(record.id) {
                Ok(Some(updated)) => println!("✓ 更新后页面: {}, 阅读次数: {}", updated.page, updated.read_times),
                Ok(None) => println!("✗ 记录不存在"),
                Err(e) => println!("✗ 查询失败: {}", e),
            }

            // 5. 按路径查找（Read by path）
            println!("\n5. 通过路径查找记录");
            match RecentDao::find_by_path_sync("/path/to/updated.pdf") {
                Ok(Some(found)) => println!("✓ 找到路径: {}", found.book_path),
                Ok(None) => println!("✗ 路径不存在"),
                Err(e) => println!("✗ 查询失败: {}", e),
            }

            // 6. 按路径更新（Update by path）
            println!("\n6. 通过路径更新记录");
            let mut path_update = Recent::new("/path/to/updated.pdf".to_string());
            path_update.progress = 50;

            match RecentDao::update_by_path_sync("/path/to/updated.pdf", &path_update) {
                Ok(()) => println!("✓ 成功按路径更新记录"),
                Err(e) => println!("✗ 更新失败: {}", e),
            }

            // 7. 删除（Delete）
            println!("\n7. 删除记录");
            match RecentDao::delete_sync(record.id) {
                Ok(()) => println!("✓ 成功删除记录"),
                Err(e) => println!("✗ 删除失败: {}", e),
            }

            // 验证删除结果
            match RecentDao::find_by_id_sync(record.id) {
                Ok(Some(_)) => println!("✗ 记录仍然存在"),
                Ok(None) => println!("✓ 确认记录已被删除"),
                Err(e) => println!("✗ 查询失败: {}", e),
            }

        },
        Err(e) => println!("✗ 插入失败: {}", e),
    }

    println!("\n=== 数据库CRUD测试完成 ===");
}

/// 运行所有数据库测试
pub async fn run_db_tests() {
    crud().await;
}
