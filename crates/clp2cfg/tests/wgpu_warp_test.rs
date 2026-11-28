#![cfg(windows)]

//! クリーンな Windows 環境（GPU なし / OpenGL 1.1 のみ / Windows Sandbox `VGpu: Disable` 相当）で、
//! `eframe` の `wgpu` バックエンドが DirectX 12 WARP（`Microsoft Basic Render Driver` /
//! `DeviceType::Cpu`）へフォールバックしてデバイス・シェーダ・レンダラ初期化できることを検証する。

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use eframe::egui_wgpu::{self, wgpu, RendererOptions, WgpuSetup};

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
    let WgpuSetup::CreateNew(setup) = eframe::NativeOptions::default().wgpu_options.wgpu_setup
    else {
        panic!("NativeOptions::default() の wgpu_setup は CreateNew であること");
    };

    let backends = setup.instance_descriptor.backends;
    assert!(
        backends.contains(wgpu::Backends::DX12),
        "NativeOptions::default() の既定バックエンドに DX12 が含まれること: {backends:?}"
    );

    let adapters = block_on(async {
        let instance = WgpuSetup::CreateNew(setup).new_instance().await;
        instance.enumerate_adapters(backends).await
    });

    assert!(
        adapters.iter().any(|a| is_dx12_warp(&a.get_info())),
        "NativeOptions::default() の既定バックエンドに DX12 WARP フォールバックが含まれること"
    );
}

#[test]
fn test_wgpu_dx12_warp_software_renderer_initializes() {
    let mut wgpu_options = eframe::NativeOptions::default().wgpu_options;
    let WgpuSetup::CreateNew(ref mut setup) = wgpu_options.wgpu_setup else {
        panic!("NativeOptions::default() の wgpu_setup は CreateNew であること");
    };
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

    let render_state = block_on(async {
        let instance = wgpu_options.wgpu_setup.new_instance().await;
        egui_wgpu::RenderState::create(&wgpu_options, &instance, None, RendererOptions::default())
            .await
    })
    .expect("DX12 WARP (Microsoft Basic Render Driver) で RenderState を初期化できること");

    let info = render_state.adapter.get_info();
    assert!(is_dx12_warp(&info), "unexpected adapter selected: {info:?}");
}
