use std::time::{Instant, Duration};
use std::thread;
use std::net::{TcpStream};
use std::io::{Read, Write};
use bit_vec::BitVec;
use std::cell::RefCell;
use embedded_hal::digital::v2::{OutputPin, InputPin};
use std::io;
use bitbang_hal::spi::MODE_0;
use bitbang_hal::spi::SPI;
use core::convert::Infallible;
use embedded_hal::timer::{CountDown, Periodic};
use void::Void;
use nb;
//use lis3dh::Lis3dh;
use lis3dsh::Lis3dsh;
use embedded_hal::blocking::delay::DelayMs;

const REGISTER_TOTAL_BITS: usize = 406;

/*
enum OCDError {
    Io(io::Error),
}
*/

pub struct DummyDelay {

}

impl DelayMs<u8> for DummyDelay {
    fn delay_ms(&mut self, ms: u8) {
        println!("sleep for {} ms", ms);
        thread::sleep(Duration::from_millis(ms.into()));
    }
}

pub struct SysTimer {
    start: Instant,
    duration: Duration,
}

impl SysTimer {
    pub fn new() -> SysTimer {
        SysTimer {
            start: Instant::now(),
            duration: Duration::from_millis(0),
        }
    }
}

impl CountDown for SysTimer {
    type Time = Duration;

    fn start<T>(&mut self, count: T)
    where
        T: Into<Self::Time>,
    {
        self.start = Instant::now();
        self.duration = count.into();
    }

    fn wait(&mut self) -> nb::Result<(), Void> {
        if (Instant::now() - self.start) >= self.duration {
            // Restart the timer to fulfill the contract by `Periodic`
            self.start = Instant::now();
            Ok(())
        } else {
            panic!("ovaj shit neki sa timerom");
        }
    }
}

impl Periodic for SysTimer {}

struct JTAGBase {
    stream: TcpStream,
    bsrin: BitVec,
    bsrout: BitVec,
}

impl JTAGBase {
    pub fn eval_drscan(&mut self) {
        match self.bsrin.set_from_hex(&openocd_rpc(&mut self.stream, &self.bsrout.to_hex()).unwrap()) {
            Ok(_) => {},
            Err(err) => {
                panic!("set_from_hex failed: {}", err);
            },
        };
    }
}


struct GPIOInput<'a> {
    bit_ctrl: usize,
    bit_input: usize,
    base: &'a RefCell<JTAGBase>,
}

impl<'a> GPIOInput<'a> {
    fn new(ctrl: usize, input: usize, base: &'a RefCell<JTAGBase>) -> GPIOInput {
        base.borrow_mut().bsrout.set(ctrl, true); //set input direction
        base.borrow_mut().bsrout.set(input, true); //no pullup?
        GPIOInput {
            base: base,
            bit_ctrl: ctrl,
            bit_input: input,
        }
    }
}

impl<'a> InputPin for GPIOInput<'a> {
    type Error = String;

    fn is_high(&self) -> Result<bool, Self::Error> {
        let mut base = self.base.borrow_mut();
        base.eval_drscan();
        Ok(base.bsrin.get(self.bit_input).unwrap())
    }

    fn is_low(&self) -> Result<bool, Self::Error> {
        let mut base = self.base.borrow_mut();
        base.eval_drscan();
        Ok(base.bsrin.get(self.bit_input).unwrap())
    }
}


struct GPIOOutput<'a> {
    bit_ctrl: usize,
    bit_output: usize,
    base: &'a RefCell<JTAGBase>,
}

impl<'a> GPIOOutput<'a> {
    fn new(ctrl: usize, output: usize, base: &'a RefCell<JTAGBase>) -> GPIOOutput {
        base.borrow_mut().bsrout.set(ctrl, false); //set output direction
        base.borrow_mut().bsrout.set(output, false); //output off
        GPIOOutput {
            base: base,
            bit_ctrl: ctrl,
            bit_output: output,
        }
    }
}

impl<'a> OutputPin for GPIOOutput<'a> {
    type Error = String;

    fn set_low(&mut self) -> Result<(), Self::Error> {
        let mut base = self.base.borrow_mut();
        base.bsrout.set(self.bit_output, false);
        //println!("stavio {} u false, bsr: {}", self.bit_output, base.bsrout.to_hex());
        base.eval_drscan();
        Ok(())
    }
    
    fn set_high(&mut self) -> Result<(), Self::Error> {
        let mut base = self.base.borrow_mut();
        base.bsrout.set(self.bit_output, true);
        //println!("stavio {} u true, bsr: {}", self.bit_output, base.bsrout.to_hex());
        base.eval_drscan();
        Ok(())
    }
}


trait OCDMagic {
    fn set_from_hex(&mut self, hex: &str) -> Result<(), String>;
    fn to_hex(&self) -> String;
}

impl OCDMagic for BitVec<u32> {
    fn set_from_hex(&mut self, hex: &str) -> Result<(), String> {
        let hex_block: Vec<&str> = hex.split(" ").collect();

        if hex_block.len() != self.blocks().count() {
            return Err("Invalid hex, split length != blocks count".to_owned());
        }

        unsafe {
            let mut hex_iter = hex_block.iter();
            for block in self.storage_mut().iter_mut() {
                *block = match u32::from_str_radix(hex_iter.next().unwrap(), 16) {
                    Ok(b) => {
                        b
                    },
                    Err(_) => {
                        return Err("u32 from str radix failao!".to_owned());
                    },
                };
            }
        }
        Ok(())
    }

    fn to_hex(&self) -> String {
        let mut cmd: String = "drscan stm32f4x.bs ".to_owned();

        //HACK: ovo radimo da bi sum_bits bio tocan, aka 22 bita za 13 byte
        let mut sum_bits = REGISTER_TOTAL_BITS as i32;
        for block in self.blocks() {
            let clen = match sum_bits < 32 {
                true => {
                    sum_bits
                },
                false => {
                    sum_bits -= 32;
                    32
                },
            };
            let part = format!("{} {:#010x} ", clen, block);
            cmd.push_str(&part);
        }
        cmd
    }
}

/*
fn drscan_command() -> String {
    let mut bv = BitVec::from_elem(REGISTER_TOTAL_BITS, false);
    
    bv.set(0, true);
    bv.set(30, true);
    bv.set(31, true);
    bv.set(63, true);
    
    println!("to_hex: {}", bv.to_hex());
    bv.set_from_hex("48924000 92000002 00924924 49648000 80000092 24924924 49000001 00492492 24924000 49000249 00012492 20000000 00a49209");

    "bokic".to_owned()
    /*
    let mut cmd: String = "drscan stm32f4x.bs ".to_owned();

    let mut sum_bits = REGISTER_TOTAL_BITS as i32;
    for block in bv.blocks() {
        let clen = match sum_bits < 32 {
            true => sum_bits,
            false => {
                sum_bits -= 32;
                32
            },
        };
        let part = format!("{} {:#010x} ", clen, block);
        cmd.push_str(&part);
    }
    cmd
    */
}
*/

fn openocd_rpc(stream: &mut TcpStream, command: &str) -> Result<String, String>{
    let tcmd = format!("{}\x1a", command);
    stream.write(tcmd.as_bytes()).unwrap();
    let mut resp = String::from("");

    loop {
        let mut inc = [0; 1];
        match stream.read_exact(&mut inc) {
            Ok(_) => {
                if inc[0] == 0x1a {
                    //println!(" -> got end");
                    return Ok(resp);
                } else {
                    //print!("{}", inc[0] as char);
                    resp.push(inc[0] as char);
                }
            },
            Err(e) => {
                println!("Failed to receive data: {}", e);
                return Err(format!("Failed to receive data: {}", e));
            }
        }
    }

}

fn main() {
    match TcpStream::connect("localhost:6666") {
        Ok(mut stream) => {
            println!("Connected to openocd RPC");

            let mut dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "init");
            println!("{:.2?} seconds for init", dr.elapsed());

            dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "poll off");
            println!("{:.2?} seconds for poll off", dr.elapsed());

            dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "irscan stm32f4x.bs 2");
            println!("{:.2?} seconds for irscan 2", dr.elapsed());

            dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "irscan stm32f4x.bs 0");
            println!("{:.2?} seconds for irscan 0", dr.elapsed());


            let mut base = JTAGBase {
                stream: stream,
                bsrout: BitVec::from_elem(416, true),
                bsrin: BitVec::from_elem(416, true),
            };

            let mut baseCell = RefCell::new(base);

            let mut tmr = SysTimer::new();
            tmr.start(Duration::from_millis(1));
            let mut mosi = GPIOOutput::new(285, 284, &baseCell);
            let mut sck = GPIOOutput::new(291, 290, &baseCell);
            let mut miso = GPIOInput::new(288, 286, &baseCell);
            let mut spi = SPI::new(MODE_0, miso, mosi, sck, tmr);

            let mut cs = GPIOOutput::new(402, 401, &baseCell); //mora ic u 0, inace je I2C mode
            cs.set_high();

            baseCell.borrow_mut().eval_drscan();

            let mut lis3dsh = Lis3dsh::new_spi(spi, cs);
            println!("whoami: {:?}", lis3dsh.who_am_i());
            println!("init: {:?}", lis3dsh.init(&mut DummyDelay{}));
            loop {
                let status = lis3dsh.status().unwrap();
                //println!("acc status -> da: {:?} or: {:?}", status.zyxda(), status.zyxor());
                println!("{:?}", lis3dsh.read_data());
                //println!("acc temp: {:?}", lis3dsh.read_temp_data());
                //thread::sleep(Duration::from_secs(1));
            }

            /*
            let mut lis3dh = match Lis3dh::new_spi(spi, cs) {
                Ok(l) => l,
                Err(e) => {
                    panic!("lis new_spi failao sa: {:?}", e);
                },
            };
            println!("whoami: {:?}", lis3dh.get_device_id());
            */

            /*
            //let mut led1 = GPIOOutput::new(165, 164, &baseCell);
            //let mut led2 = GPIOOutput::new(168, 167, &baseCell);
            //let mut led3 = GPIOOutput::new(162, 161, &baseCell);
            let mut btn = GPIOInput::new(318, 316, &baseCell);
            baseCell.borrow_mut().eval_drscan();
            println!("prvi drscan, sve off");
            let slp = Duration::from_secs(1);
            thread::sleep(slp);

            for i in 0..50 {
                /*
                led1.set_high();
                led2.set_high();
                led3.set_high();
                println!("led high");
                thread::sleep(slp);
                led1.set_low();
                led2.set_low();
                led3.set_low();
                println!("led low");
                thread::sleep(slp);
                */
                if btn.is_high().unwrap() {
                    println!("btn high");
                } else {
                    println!("btn low");
                }
                thread::sleep(slp);
            }
            */

            /*
            let mut dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "init");
            println!("{:.2?} seconds for init", dr.elapsed());

            dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "poll off");
            println!("{:.2?} seconds for poll off", dr.elapsed());

            dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "irscan stm32f4x.bs 2");
            println!("{:.2?} seconds for irscan 2", dr.elapsed());

            dr = std::time::Instant::now();
            openocd_rpc(&mut stream, "irscan stm32f4x.bs 0");
            println!("{:.2?} seconds for irscan 0", dr.elapsed());


            //HACK jer ovaj 13-byte mora bit full FF, inace ne radi
            let mut bsrout = BitVec::from_elem(416, true);
            let mut bsrin = BitVec::from_elem(416, true);

            dr = std::time::Instant::now();
            match bsrin.set_from_hex(&openocd_rpc(&mut stream, &bsrout.to_hex()).unwrap()) {
                Ok(_) => {},
                Err(err) => {
                    panic!("set_from_hex failed: {}", err);
                },
            };
            println!("{:.2?} seconds for drscan", dr.elapsed());

            let slp = Duration::from_secs(5);
            thread::sleep(slp);

            bsrout.set(165, false);
            bsrout.set(167, true);

            dr = std::time::Instant::now();
            match bsrin.set_from_hex(&openocd_rpc(&mut stream, &bsrout.to_hex()).unwrap()) {
                Ok(_) => {},
                Err(err) => {
                    panic!("set_from_hex failed: {}", err);
                },
            };
            println!("{:.2?} seconds for drscan", dr.elapsed());

            thread::sleep(slp);

            bsrout.set(162, false);
            bsrout.set(164, true);

            dr = std::time::Instant::now();
            match bsrin.set_from_hex(&openocd_rpc(&mut stream, &bsrout.to_hex()).unwrap()) {
                Ok(_) => {},
                Err(err) => {
                    panic!("set_from_hex failed: {}", err);
                },
            };
            println!("{:.2?} seconds for drscan", dr.elapsed());

            thread::sleep(slp);

            */ 
        },
        Err(e) => {
            println!("Failed to connect: {}", e);
        }
    }
    println!("Terminated.");
}
