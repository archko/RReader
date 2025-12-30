#![allow(unused)]
#![allow(dead_code)]

pub mod app_handler;
pub mod cache;
pub mod controllers;
pub mod dao;
pub mod decoder;
pub mod entity;
pub mod page;
pub mod tts;
pub mod ui;

// 导出Slint生成的类型
slint::include_modules!();
