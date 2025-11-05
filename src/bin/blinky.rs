#![no_std]
#![no_main]

#[cfg(feature = "defmt")]
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::tcp::TcpSocket;
use embassy_net::{Ipv4Address, Ipv4Cidr, StackResources};
use embassy_stm32::eth::{Ethernet, GenericPhy, PacketQueue};
use embassy_stm32::peripherals::ETH;
use embassy_stm32::rng::Rng;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::spi::{Config, Spi};
use embassy_stm32::time::Hertz;
use embassy_stm32::usart::BufferedUart;
use embassy_stm32::{bind_interrupts, peripherals, usart, rng, eth};
use embassy_time::Timer;
use embedded_io_async::Write;
use spi_memory::series25::Flash;
use spi_memory::Read;
use static_cell::StaticCell;
use embassy_stm32::flash::{Flash as STM32Flash, WRITE_SIZE};
use embassy_boot_stm32::{AlignedBuffer, FirmwareUpdater, FirmwareUpdaterConfig};
use embassy_sync::blocking_mutex::Mutex;
use embassy_embedded_hal::adapter::BlockingAsync;
use embassy_boot_stm32::BlockingFirmwareUpdater;
use core::cell::RefCell;
use heapless::Vec;
#[cfg(feature = "defmt")]
use {defmt_rtt as _};
//use panic_reset as _;
use panic_probe as _;
//const SIZE_IN_BYTES: u32 = (64 * 1024 * 1024) / 8;

bind_interrupts!(struct Irqs_Eth {
    ETH => eth::InterruptHandler;
    RNG => rng::InterruptHandler<peripherals::RNG>;
});

type Device = Ethernet<'static, ETH, GenericPhy>;

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, Device>) -> ! {
    runner.run().await
}
bind_interrupts!(struct Irqs {
    USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    //let p = embassy_stm32::init(Default::default());
    let config = {
        use embassy_stm32::rcc::*;

        let mut config = embassy_stm32::Config::default();
        config.rcc.hse = Some(Hse {
            freq: Hertz::mhz(25),
            mode: HseMode::Oscillator,
        });
        config.rcc.pll_src = PllSource::HSE;
        config.rcc.pll = Some(Pll {
            prediv: PllPreDiv::DIV25,
            mul: PllMul::MUL336,
            divp: Some(PllPDiv::DIV2),
            divq: Some(PllQDiv::DIV7),
            divr: None,
        });
        config.rcc.sys = Sysclk::PLL1_P;

        config.rcc.ahb_pre = AHBPrescaler::DIV1;
        config.rcc.apb1_pre = APBPrescaler::DIV4;
        config.rcc.apb2_pre = APBPrescaler::DIV2;

        // reference your chip's manual for proper clock settings; this config
        // is recommended for a 32 bit frame at 48 kHz sample rate
        config.rcc.plli2s = Some(Pll {
            prediv: PllPreDiv::DIV25,
            mul: PllMul::MUL336,
            divp: None,
            divq: None,
            divr: Some(PllRDiv::DIV5),
        });
        config.enable_debug_during_sleep = true;

        config
    };
    let p = embassy_stm32::init(config);
#[cfg(feature = "defmt")]
    info!("Hello World!");

    let mut led = Output::new(p.PC15, Level::High, Speed::Low);

    let mut spi_config = Config::default();
    spi_config.frequency = Hertz(1_000_000);

    let spi = Spi::new_blocking(p.SPI2, p.PB10, p.PB15, p.PB14, spi_config);

    let cs = Output::new(p.PB9, Level::High, Speed::VeryHigh);
    let mut flash = Flash::init(spi, cs).unwrap();
    let id = flash.read_jedec_id().unwrap();
#[cfg(feature = "defmt")]
    info!(
        "spi flash id {:?} {:?} {:?}",
        id.mfr_code(),
        id.continuation_count(),
        id.device_id()
    );

    let addr = 0;
    const BUF: usize = 32;
    let mut buf = [0; BUF];
    //let buf1 = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];

    //while addr < SIZE_IN_BYTES {
    //flash.write(buf1, addr, 1);
    flash.read(addr, &mut buf).unwrap();

    let mut config = usart::Config::default();
    config.baudrate = 115_200;
    static TX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    let tx_buf = &mut TX_BUF.init([0; 128])[..];
    static RX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    let rx_buf = &mut RX_BUF.init([0; 128])[..];
    let usart = BufferedUart::new(p.USART1, p.PA10, p.PA9, tx_buf, rx_buf, Irqs, config).unwrap();
    let (mut usr_tx, _usr_rx) = usart.split();
#[cfg(feature = "defmt")]
    info!("{:?}", buf);
    let _ = usr_tx.write_all(&buf).await;
    //    addr += BUF as u32;
    //}
    let layout = STM32Flash::new_blocking(p.FLASH).into_blocking_regions();
    let flash_state = Mutex::new(RefCell::new(layout.bank1_region1));
    let flash_active_dfu = Mutex::new(RefCell::new(layout.bank1_region3));
    let config = FirmwareUpdaterConfig::from_linkerfile_blocking(&flash_active_dfu, &flash_state);
    let mut magic = AlignedBuffer([0; WRITE_SIZE]);
    let mut firmware_state = BlockingFirmwareUpdater::new(config, &mut magic.0);
    firmware_state.mark_booted().expect("Failed to mark booted");
    // Generate random seed.
    let mut phy_rst = Output::new(p.PD0, Level::High, Speed::Low);
    phy_rst.set_high();
    let mut rng = Rng::new(p.RNG, Irqs_Eth);
    let mut seed = [0; 8];
    let _ = rng.async_fill_bytes(&mut seed).await;
    let seed = u64::from_le_bytes(seed);

    let mac_addr = [0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF];

    static PACKETS: StaticCell<PacketQueue<4, 4>> = StaticCell::new();
    let device = Ethernet::new(
        PACKETS.init(PacketQueue::<4, 4>::new()),
        p.ETH,
        Irqs_Eth,
        p.PA1,
        p.PA2,
        p.PC1,
        p.PA7,
        p.PC4,
        p.PC5,
        p.PB12,
        p.PB13,
        p.PB11,
        GenericPhy::new_auto(),
        mac_addr,
    );

    //let config = embassy_net::Config::dhcpv4(Default::default());
    let config = embassy_net::Config::ipv4_static(embassy_net::StaticConfigV4 {
        address: Ipv4Cidr::new(Ipv4Address::new(192, 168, 2, 34), 24),
        dns_servers: Vec::new(),
        gateway: Some(Ipv4Address::new(192, 168, 2, 1)),
    });

    // Init network stack
    static RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(device, config, RESOURCES.init(StackResources::new()), seed);

    // Launch network task
    spawner.spawn(net_task(runner).unwrap());

    // Ensure DHCP configuration is up before trying connect
    stack.wait_config_up().await;

    #[cfg(feature = "defmt")]
    info!("Network task initialized");

    // Then we can use it!
    let mut rx_buffer = [0; 4096];
    let mut tx_buffer = [0; 4096];
    let mut buf = [0; 4096];

    loop {
        let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
        socket.set_timeout(Some(embassy_time::Duration::from_secs(10)));

        #[cfg(feature = "defmt")]
        info!("Listening on TCP:1234...");
        if let Err(e) = socket.accept(1234).await {
            #[cfg(feature = "defmt")]
            warn!("accept error: {:?}", e);
            continue;
        }

        #[cfg(feature = "defmt")]
        info!("Received connection from {:?}", socket.remote_endpoint());

        loop {
            let n = match socket.read(&mut buf).await {
                Ok(0) => {
                    #[cfg(feature = "defmt")]
                    warn!("read EOF");
                    break;
                }
                Ok(n) => n,
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    warn!("read error: {:?}", e);
                    break;
                }
            };

            led.toggle();
            #[cfg(feature = "defmt")]
            info!("rxd {:02x}", &buf[..n]);

            match socket.write_all(&buf[..n]).await {
                Ok(()) => {}
                Err(e) => {
                    #[cfg(feature = "defmt")]
                    warn!("write error: {:?}", e);
                    break;
                }
            };
        }
    }
/*    loop {
#[cfg(feature = "defmt")]
        info!("high");
        usr_tx.write_all("high\r\n".as_bytes()).await;
        led.set_high();
        Timer::after_millis(100).await;

#[cfg(feature = "defmt")]
        info!("low");
        usr_tx.write_all("low\r\n".as_bytes()).await;
        led.set_low();
        Timer::after_millis(100).await;
    }*/
}
