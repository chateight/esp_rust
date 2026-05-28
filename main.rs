#![no_std]
#![no_main]
//
// GPIOの割り当て
// sensor right -> GPIO2 (ADC1_CH2)
// sensor left -> GPIO3 (ADC1_CH3)
// motor right -> GPIO9
// motor left -> GPIO7
// motor direction right -> GPIO8
// motor direction left -> GPIO6

use embassy_executor::Spawner;
use embassy_net::{
    udp::{PacketMetadata, UdpSocket},
    IpEndpoint, Runner, Stack, StackResources,
};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_backtrace as _;
use esp_hal::{
    analog::adc::{Adc, AdcCalLine, AdcConfig, Attenuation},
    clock::CpuClock,
    interrupt::software::SoftwareInterruptControl,
    ram,
    rng::Rng,
    timer::timg::TimerGroup,
};

use esp_hal::gpio::{Level, Output, OutputConfig};

use esp_println::println;
use esp_radio::wifi::{sta::StationConfig, Config, ControllerConfig, Interface, WifiController};

use core::sync::atomic::{AtomicBool, AtomicI16, Ordering};

extern crate alloc;
use alloc::format;

static LEFT_POWER: AtomicI16 = AtomicI16::new(25);
static RIGHT_POWER: AtomicI16 = AtomicI16::new(25);

static GO_STOP: AtomicBool = AtomicBool::new(false);

static POSE_RIGHT_POWER: AtomicI16 = AtomicI16::new(0);
static POSE_LEFT_POWER: AtomicI16 = AtomicI16::new(0);

esp_bootloader_esp_idf::esp_app_desc!();

// static領域確保用マクロ
macro_rules! mk_static {
    ($t:ty,$val:expr) => {{
        static STATIC_CELL: static_cell::StaticCell<$t> = static_cell::StaticCell::new();
        let x = STATIC_CELL.uninit().write(($val));
        x
    }};
}

const SSID: &str = env!("SSID");
const PASSWORD: &str = env!("PASSWORD");

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    println!("Target SSID: <{}>", SSID);

    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // ヒープ初期化
    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let station_config = Config::Station(
        StationConfig::default()
            .with_ssid(SSID)
            .with_password(PASSWORD.into()),
    );

    let (controller, interfaces) = esp_radio::wifi::new(
        peripherals.WIFI,
        ControllerConfig::default().with_initial_config(station_config),
    )
    .unwrap();

    let config = embassy_net::Config::dhcpv4(Default::default());
    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // ネットワークスタック初期化
    let (stack, runner) = embassy_net::new(
        interfaces.station,
        config,
        mk_static!(StackResources<8>, StackResources::<8>::new()),
        seed,
    );

    // motor task設定
    let mut motor_pin_right = Output::new(peripherals.GPIO9, Level::Low, OutputConfig::default());
    let mut motor_pin_left = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());
    let mut motor_direction_right =
        Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    let mut motor_direction_left =
        Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());
    motor_direction_right.set_high(); // 右モーターの回転方向を設定
    motor_direction_left.set_high(); // 左モーターの回転方向を設定
    motor_pin_right.set_low(); // 初期状態は停止
    motor_pin_left.set_low(); // 初期状態は停止

    // 各タスクの起動
    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(udp_server_task(stack).unwrap());
    spawner.spawn(udp_sender_task(stack).unwrap());
    spawner.spawn(adc_task(peripherals.ADC1, peripherals.GPIO2, peripherals.GPIO3).unwrap());
    spawner.spawn(motor_task_right(motor_pin_right).unwrap());
    spawner.spawn(motor_task_left(motor_pin_left).unwrap());

    // メインはIP取得を待って報告するだけ
    stack.wait_config_up().await;
    if let Some(config) = stack.config_v4() {
        println!("UDP Server listening on {}:5000", config.address);
    }

    loop {
        Timer::after(Duration::from_secs(60)).await;
    }
}

#[embassy_executor::task]
async fn motor_task_right(mut pin: Output<'static>) {
    const PWM_PERIOD_US: u64 = 2000;
    Timer::after(Duration::from_millis(1000)).await; // 起動後すぐにモーターが動かないように1秒待機

    loop {
        // 0〜100想定
        let power = RIGHT_POWER.load(Ordering::Relaxed);

        let high_time = (PWM_PERIOD_US * power as u64) / 100;

        let low_time = PWM_PERIOD_US - high_time;

        if high_time > 0 {
            pin.set_high();

            Timer::after(Duration::from_micros(high_time)).await;
        }

        if low_time > 0 {
            pin.set_low();

            Timer::after(Duration::from_micros(low_time)).await;
        }
    }
}

#[embassy_executor::task]
async fn motor_task_left(mut pin: Output<'static>) {
    const PWM_PERIOD_US: u64 = 2000;
    Timer::after(Duration::from_millis(1000)).await; // 起動後すぐにモーターが動かないように1秒待機

    loop {
        // 0〜100想定
        let power = LEFT_POWER.load(Ordering::Relaxed);

        let high_time = (PWM_PERIOD_US * power as u64) / 100;

        let low_time = PWM_PERIOD_US - high_time;

        if high_time > 0 {
            pin.set_high();

            Timer::after(Duration::from_micros(high_time)).await;
        }

        if low_time > 0 {
            pin.set_low();

            Timer::after(Duration::from_micros(low_time)).await;
        }
    }
}

#[embassy_executor::task]
async fn adc_task(
    adc_peripheral: esp_hal::peripherals::ADC1<'static>,
    gpio2: esp_hal::peripherals::GPIO2<'static>,
    gpio3: esp_hal::peripherals::GPIO3<'static>,
) {
    let mut adc_config = AdcConfig::new();

    let mut adc_pin_right =
        adc_config.enable_pin_with_cal::<_, AdcCalLine<_>>(gpio2, Attenuation::_11dB);
    let mut adc_pin_left =
        adc_config.enable_pin_with_cal::<_, AdcCalLine<_>>(gpio3, Attenuation::_11dB);

    let mut adc = Adc::new(adc_peripheral, adc_config);

    const MOTOR_COEFFICIENT: f64 = 1.07; // 右モーターの出力を少し強くするための係数(ajust as needed to balance left and right motors)

    loop {
        let go = GO_STOP.load(Ordering::Relaxed);
        if !go {
            // if GO_STOP is false, skip reading ADC and set motors to idle
            RIGHT_POWER.store(0, Ordering::Relaxed);
            LEFT_POWER.store(0, Ordering::Relaxed);
            Timer::after(Duration::from_millis(10)).await; // idle 状態でのCPU負荷を下げるために少し待機
            continue;
        }

        let value_right = loop {
            match adc.read_oneshot(&mut adc_pin_right) {
                Ok(v) => break v,
                Err(_) => continue,
            }
        };

        let value_left = loop {
            match adc.read_oneshot(&mut adc_pin_left) {
                Ok(v) => break v,
                Err(_) => continue,
            }
        };

        //
        // motor control logic
        //
        let value_right = (value_right as i16) / 40; // 0-4095を0〜100に変換
        let value_left = (value_left as i16) / 40; // 0-4095を0〜100に変換

        let pow_coeff = 2.0;
        let bp = 23.0; // ベースのモーター出力（0~100）
        let idle_power = 5.0; // 最小出力（0~100）

        let diff = value_right - value_left;
        let sum = value_right + value_left + 1; // ゼロ割り防止のために1加算
        let ratio = ((diff as f32) * pow_coeff / (sum as f32)).clamp(1.0, 2.0);
        let mut motor_power = (ratio * bp) as i16;
        motor_power = motor_power.clamp(0, 100); // モーター出力を0〜100の範囲に制限
        let corrected = ((motor_power as f64) * MOTOR_COEFFICIENT).clamp(0.0, 100.0); // 左右のモーター出力のバランスを取るための係数（必要に応じて調整）

        if diff > 5 {
            // センサーの右が明るいときは右を強く、左を弱く
            RIGHT_POWER.store(corrected as i16, Ordering::Relaxed);
            LEFT_POWER.store(idle_power as i16, Ordering::Relaxed);
            POSE_RIGHT_POWER.store(motor_power as i16, Ordering::Relaxed);
            POSE_LEFT_POWER.store(idle_power as i16, Ordering::Relaxed);
        } else if diff < -5 {
            // センサーの左が明るいときは左を強く、右を弱く
            RIGHT_POWER.store(idle_power as i16, Ordering::Relaxed);
            LEFT_POWER.store(motor_power as i16, Ordering::Relaxed);
            POSE_RIGHT_POWER.store(idle_power as i16, Ordering::Relaxed);
            POSE_LEFT_POWER.store(motor_power as i16, Ordering::Relaxed);
        } else {
            // ほぼ同じ明るさのときは同じ出力
            RIGHT_POWER.store(corrected as i16, Ordering::Relaxed);
            LEFT_POWER.store(motor_power, Ordering::Relaxed);
            POSE_RIGHT_POWER.store(motor_power as i16, Ordering::Relaxed);
            POSE_LEFT_POWER.store(motor_power, Ordering::Relaxed);
        }

        //println!("ADC Raw Value: {}, {}", value_right, value_left);

        Timer::after(Duration::from_millis(10)).await;
    }
}

#[embassy_executor::task]
async fn udp_server_task(stack: Stack<'static>) {
    // UDP受信に必要なメタデータと受信バッファの準備
    let mut rx_meta = [PacketMetadata::EMPTY; 4];
    let mut rx_payload = [0u8; 512];
    let mut tx_meta = [PacketMetadata::EMPTY; 4];
    let mut tx_payload = [0u8; 512];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_payload,
        &mut tx_meta,
        &mut tx_payload,
    );

    // ポート5000で待ち受け開始
    if let Err(e) = socket.bind(5000) {
        println!("UDP bind error: {:?}", e);
        return;
    }

    let mut buf = [0u8; 512];
    loop {
        // メッセージが来るまで非同期で待機（他タスクを開放）
        match socket.recv_from(&mut buf).await {
            Ok((n, _endpoint)) => {
                let data = &buf[..n];
                if let Ok(s) = core::str::from_utf8(data) {
                    // "0" や "1" など、ライントレーサーのコマンドとして
                    let command = s.trim();
                    if command == "0" || command == "1" {
                        if command == "1" {
                            GO_STOP.store(true, Ordering::Relaxed);
                        } else if command == "0" {
                            GO_STOP.store(false, Ordering::Relaxed);
                        }
                        println!("COMMAND: {}", command);
                    } else {
                        // x などの儀式パケットが来ても、何もしない（continue）
                        continue;
                    }
                }
            }
            Err(e) => println!("UDP recv error: {:?}", e),
        }
    }
}

#[embassy_executor::task]
async fn udp_sender_task(stack: Stack<'static>) {
    // 送信用socket用バッファ
    let mut rx_meta = [PacketMetadata::EMPTY; 1];
    let mut rx_payload = [0u8; 64];
    let mut tx_meta = [PacketMetadata::EMPTY; 1];
    let mut tx_payload = [0u8; 256];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_payload,
        &mut tx_meta,
        &mut tx_payload,
    );

    // 適当なローカルポートbind
    socket.bind(5001).unwrap();

    // 送信先PCのIPとポート
    let remote = IpEndpoint::new(embassy_net::IpAddress::v4(192, 168, 1, 11), 5001);

    let mut accumulation: i16 = 0;

    loop {
        let go = GO_STOP.load(Ordering::Relaxed);
        if go == true {
            let mut accumulated_left: i16 = 0;
            let mut accumulated_right: i16 = 0;

            for _ in 0..10 {
                Timer::after(Duration::from_millis(10)).await;  // 10回分のモーター制御出力を累積してright/leftの差分を作成する　

                let left = POSE_LEFT_POWER.load(Ordering::Relaxed);
                let right = POSE_RIGHT_POWER.load(Ordering::Relaxed);
                accumulated_left += left;
                accumulated_right += right;
            }
            let accumulated_diff_part = accumulated_left - accumulated_right;
            accumulation += accumulated_diff_part;
            let msg = format!("{},{},{},{}\n", accumulation, accumulated_left, accumulated_right, accumulated_diff_part);

            match socket.send_to(msg.as_bytes(), remote).await {
                Ok(_) => {
                    //println!("UDP SEND: {}", msg);
                }
                Err(e) => {
                    println!("UDP send error: {:?}", e);
                }
            }
        } else {
            // GO_STOPがfalseのときは、UDP送信を行わずに少し待機してループを続ける
            Timer::after(Duration::from_millis(100)).await;
        }
    }
}

#[embassy_executor::task]
async fn connection(mut controller: WifiController<'static>) {
    loop {
        if let Err(e) = controller.connect_async().await {
            println!("Wifi connect failed: {:?}", e);
            Timer::after(Duration::from_millis(5000)).await;
            continue;
        }
        println!("Wifi connected!");
        controller.wait_for_disconnect_async().await.ok();
        println!("Wifi disconnected!");
    }
}

#[embassy_executor::task]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await
}
