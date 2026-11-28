#![cfg(windows)]

//! クリーンな Windows 環境（GPU なし / OpenGL 1.1 のみ / Windows Sandbox `VGpu: Disable` 相当）で、
//! `eframe` の `wgpu` バックエンドが DirectX 12 WARP（`Microsoft Basic Render Driver` /
//! `DeviceType::Cpu`）へフォールバックしてデバイス・シェーダ・レンダラ初期化できることを検証する。

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use eframe::egui_wgpu::{
    self, wgpu, RendererOptions, WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew,
};

struct ThreadWaker(Thread);

impl Wake for ThreadWaker {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

/// 追加クレートなしで `egui_wgpu` の Future を完了まで待機する簡易ランナー。
fn block_on<F: Future>(future: F) -> F::Output {
    let waker = Waker::from(Arc::new(ThreadWaker(thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut future = pin!(future);
    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => thread::park(),
        }
    }
}

fn is_dx12_warp(info: &wgpu::AdapterInfo) -> bool {
    info.backend == wgpu::Backend::Dx12
        && info.device_type == wgpu::DeviceType::Cpu
        && info.name.contains("Microsoft Basic Render Driver")
}

#[test]
fn test_wgpu_default_backends_expose_dx12_warp_fallback() {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    // 外部の WGPU_BACKEND 環境変数に左右されず、eframe 既定の PRIMARY | GL 構成を検証する。
    setup.instance_descriptor.backends = wgpu::Backends::PRIMARY | wgpu::Backends::GL;

    let adapters = block_on(async {
        let instance = WgpuSetup::CreateNew(setup).new_instance().await;
        instance
            .enumerate_adapters(wgpu::Backends::PRIMARY | wgpu::Backends::GL)
            .await
    });

    assert!(
        adapters.iter().any(|a| is_dx12_warp(&a.get_info())),
        "既定バックエンド（PRIMARY | GL）に DX12 WARP フォールバックが含まれること"
    );
}

#[test]
fn test_wgpu_dx12_warp_software_renderer_initializes() {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    setup.instance_descriptor.backends = wgpu::Backends::DX12;
    setup.native_adapter_selector = Some(Arc::new(|adapters, compatible_surface| {
        adapters
            .iter()
            .find(|adapter| {
                is_dx12_warp(&adapter.get_info())
                    && compatible_surface
                        .is_none_or(|surface| adapter.is_surface_supported(surface))
            })
            .cloned()
            .ok_or_else(|| "DX12 WARP adapter not found".to_owned())
    }));

    let config = WgpuConfiguration {
        wgpu_setup: WgpuSetup::CreateNew(setup),
        ..Default::default()
    };

    let render_state = block_on(async {
        let instance = config.wgpu_setup.new_instance().await;
        egui_wgpu::RenderState::create(&config, &instance, None, RendererOptions::default()).await
    })
    .expect("DX12 WARP (Microsoft Basic Render Driver) で RenderState を初期化できること");

    let info = render_state.adapter.get_info();
    assert!(is_dx12_warp(&info), "unexpected adapter selected: {info:?}");
}
