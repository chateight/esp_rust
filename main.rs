#![no_std]
#![no_main]
//
// GPIOの割り当て
// sensor right -> GPIO2 (ADC1_CH2)
// sensor left -> GPIO3 (ADC1_CH3)
// motor right -> GPIO6
// motor left -> GPIO7
// motor direction right -> GPIO8
// motor direction left -> GPIO9

use embassy_executor::Spawner;
use embassy_net::{
    udp::{PacketMetadata, UdpSocket},
    Runner, Stack, StackResources,
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

use core::sync::atomic::{AtomicI16, Ordering};

static LEFT_POWER: AtomicI16 = AtomicI16::new(50);
static RIGHT_POWER: AtomicI16 = AtomicI16::new(50);

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
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );

    // motor task設定
    let motor_pin_right = Output::new(peripherals.GPIO6, Level::Low, OutputConfig::default());
    let motor_pin_left = Output::new(peripherals.GPIO7, Level::Low, OutputConfig::default());
    let mut motor_direction_right = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    let mut motor_direction_left = Output::new(peripherals.GPIO9, Level::Low, OutputConfig::default());
    motor_direction_right.set_high(); // 右モーターの回転方向を設定
    motor_direction_left.set_high(); // 左モーターの回転方向を設定

    // 各タスクの起動
    spawner.spawn(connection(controller).unwrap());
    spawner.spawn(net_task(runner).unwrap());
    spawner.spawn(udp_server_task(stack).unwrap());
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
    const PWM_PERIOD_US: u64 = 1000;

    loop {
        // 0〜100想定
        let power = RIGHT_POWER.load(Ordering::Relaxed);

        let power = power.clamp(0, 100);

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
    const PWM_PERIOD_US: u64 = 1000;

    loop {
        // 0〜100想定
        let power = LEFT_POWER.load(Ordering::Relaxed);

        let power = power.clamp(0, 100);

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

    loop {
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

        let pow_coeff = 3.0;
        let bp = 50.0; // ベースのモーター出力（0~100）
        let idle_power = 20.0; // 最小出力（0~100）

        let diff = value_right - value_left;
        let sum = value_right + value_left;
        let ratio = ((diff as f32)* pow_coeff / (sum as f32)).clamp(1.0, 2.0);
        let motor_power = (ratio * bp) as i16;

        if diff > 10 {
            // センサーの右が明るいときは右を強く、左を弱く
            RIGHT_POWER.store(motor_power, Ordering::Relaxed);
            LEFT_POWER.store(idle_power as i16, Ordering::Relaxed);
        } else if diff < -10 {
            // センサーの左が明るいときは左を強く、右を弱く
            RIGHT_POWER.store(idle_power as i16, Ordering::Relaxed);
            LEFT_POWER.store(motor_power, Ordering::Relaxed);
        } else {
            // ほぼ同じ明るさのときは同じ出力
            RIGHT_POWER.store(motor_power, Ordering::Relaxed);
            LEFT_POWER.store(motor_power, Ordering::Relaxed);
        }

        println!("ADC Raw Value: {}, {}", value_right, value_left);

        Timer::after(Duration::from_millis(100)).await;
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
