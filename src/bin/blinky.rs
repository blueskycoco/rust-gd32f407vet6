#![no_std]
#![no_main]

use defmt::*;
use embassy_executor::Spawner;
use embassy_stm32::gpio::{Level, Output, Speed};
use embassy_stm32::spi::{Config, Spi};
use embassy_stm32::time::Hertz;
use embassy_stm32::usart::BufferedUart;
use embassy_stm32::{bind_interrupts, peripherals, usart};
use embassy_time::Timer;
use embedded_io_async::Write;
use spi_memory::series25::Flash;
use spi_memory::Read;
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

//const SIZE_IN_BYTES: u32 = (64 * 1024 * 1024) / 8;

bind_interrupts!(struct Irqs {
    USART1 => usart::BufferedInterruptHandler<peripherals::USART1>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
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
    info!("Hello World!");

    let mut led = Output::new(p.PC15, Level::High, Speed::Low);

    let mut spi_config = Config::default();
    spi_config.frequency = Hertz(1_000_000);

    let spi = Spi::new_blocking(p.SPI2, p.PB10, p.PB15, p.PB14, spi_config);

    let cs = Output::new(p.PB9, Level::High, Speed::VeryHigh);
    let mut flash = Flash::init(spi, cs).unwrap();
    let id = flash.read_jedec_id().unwrap();
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
    config.baudrate = 921_600;
    static TX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    let tx_buf = &mut TX_BUF.init([0; 128])[..];
    static RX_BUF: StaticCell<[u8; 128]> = StaticCell::new();
    let rx_buf = &mut RX_BUF.init([0; 128])[..];
    let usart = BufferedUart::new(p.USART1, p.PA10, p.PA9, tx_buf, rx_buf, Irqs, config).unwrap();
    let (mut usr_tx, _usr_rx) = usart.split();
    info!("{:?}", buf);
    let _ = usr_tx.write_all(&buf).await;
    //    addr += BUF as u32;
    //}
    loop {
        //info!("high");
        usr_tx.write_all("high\r\n".as_bytes()).await;
        led.set_high();
        Timer::after_millis(300).await;

        //info!("low");
        usr_tx.write_all("low\r\n".as_bytes()).await;
        led.set_low();
        Timer::after_millis(300).await;
    }
}
