//! This application is a Monte Carlo implementation of a closed system discrete time Markov chain
//! with 2 servers. The goal of this program is to validate my analytical model results.
//!
//! In designing this, I had two primary directions to go:
//! 1. Generate the events before the simulation starts, then move directly to the tick they happen
//!    in. This doesn't feel correct, for some reason, so I didn't do it this way.
//! 2. Skip to the next event,

use clap::Parser;
use rand::{SeedableRng, random, rngs::SmallRng};
use rand_distr::{Distribution, Geometric};
use std::{cmp::Reverse, collections::BinaryHeap, ops::Deref};

/// Monte Carlo simulation of a machine shop with N machines and a 2-person repair center.
/// All times are in minutes (one tick = one minute).
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Number of machines (N)
    #[arg(short = 'n', long, default_value_t = 10)]
    machines: u32,

    /// Mean repair time P, in minutes
    /// Default of 8 hours in minutes
    #[arg(short = 'p', long, default_value_t = 60 * 8)]
    repair_time: u64,

    /// Mean time from a repair until the next failure R, in minutes
    /// Default of 2 weeks in minutes
    #[arg(short = 'r', long, default_value_t = 60 * 24 * 7 * 2)]
    interbreak_time: u64,

    /// Simulation duration T, in minutes
    /// Default of 1 year of runtime
    #[arg(short = 't', long, default_value_t = 60 * 24 * 30 * 12)]
    duration: u64,

    /// RNG seed (random if omitted)
    #[arg(short = 's', long)]
    seed: Option<u64>,
}

#[derive(Debug, Ord, PartialEq, PartialOrd, Eq, Copy, Clone)]
#[repr(transparent)]
struct Tick(u64);

impl Deref for Tick {
    type Target = u64;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// What happens at a scheduled tick
#[derive(Debug, Ord, PartialEq, PartialOrd, Eq, Copy, Clone)]
enum Event {
    /// The computer with this id breaks
    Break(u32),
    /// The server with this index finishes its current repair
    RepairDone(usize),
}

#[derive(Debug)]
struct Computer {
    id: u32,

    /// The tick this computer breaks (or broke) at
    broken_until: Option<Tick>,

    /// The tick its repair finishes, sampled once a server takes it
    fixed_at: Option<Tick>,

    /// The tick that assigns this computer to a server
    taken_at: Option<Tick>,

    /// each computer has its own RNG source for its distribution sampling
    rng: SmallRng,

    /// The distribution that decides when a computer breaks
    /// NOTE: Littles' law is independent of distribution, and so in theory we could put any
    /// distribution we want in here. However, the Distribution trait is not dyn compatible and so I
    /// just picked one and hardcoded it.
    broken_dist: Geometric,

    /// The distribution that decides how many ticks a repair takes
    /// NOTE: See above
    fix_dist: Geometric,
}

impl Computer {
    pub fn new(id: u32, args: &Args, rng: &mut SmallRng) -> Self {
        Self {
            id,
            broken_until: None,
            fixed_at: None,
            taken_at: None,
            broken_dist: Geometric::new(1.0 / args.interbreak_time as f64).unwrap(),
            fix_dist: Geometric::new(1.0 / args.repair_time as f64).unwrap(),
            rng: SmallRng::from_rng(rng),
        }
    }

    /// Samples how many ticks until this computer next breaks
    /// NOTE: from the rand_dist Geometric docs:
    /// ```
    /// This is the probability distribution of the number of failures (bounded to [0, u64::MAX])
    /// before the first success in a series of Bernoulli trials, where the probability of success on
    /// each trial is p.
    ///```
    ///Therefore we add 1 so the mean is exactly 1/p
    pub fn sample_break(&mut self) -> u64 {
        // rand_distr's Geometric counts failures before the first success (support 0, 1, ...),
        // so add 1 to get the trial count, whose mean is exactly 1/p
        1 + self.broken_dist.sample(&mut self.rng)
    }

    /// Samples how many ticks a repair of this computer takes
    /// NOTE: See `sample_break` docs on why we add 1 here.
    pub fn sample_fix(&mut self) -> u64 {
        // rand_distr's Geometric counts failures before the first success (support 0, 1, ...),
        // so add 1 to get the trial count, whose mean is exactly 1/p
        1 + self.fix_dist.sample(&mut self.rng)
    }

    pub fn set_taken_at(mut self, tick: Tick) -> Self {
        self.taken_at = Some(tick);
        self
    }

    pub fn set_fixed_at(mut self, tick: Tick) -> Self {
        self.fixed_at = Some(tick);
        self
    }

    pub fn set_broken_at(mut self, tick: Tick) -> Self {
        self.broken_until = Some(tick);
        self
    }

    pub fn broken_until(&self) -> Option<Tick> {
        self.broken_until
    }

    pub fn id(&self) -> u32 {
        self.id
    }
}

#[derive(Default, Debug)]
struct Server {
    current_item: Option<Computer>,
}

impl Server {
    /// Whether our server is idle. In theory this wasn't really needed as we know through the event
    /// enum that a particular server is done with a computer.
    pub fn is_idle(&self) -> bool {
        self.current_item.is_none()
    }

    /// Takes the computer that finishes on this tick, returning it so it can be returned to the
    /// computer list.
    fn finish_computer(&mut self) -> Option<Computer> {
        std::mem::take(&mut self.current_item)
    }
}

struct Simulation {
    servers: [Server; 2],
    computers: Vec<Option<Computer>>,
    fix_queue: Vec<Computer>,
    duration: u64,

    /// The ticks our events happen at, so we can skip straight to them
    /// Reverse is to convert the by-default max heap into a minimum one
    events: BinaryHeap<Reverse<(Tick, Event)>>,

    /// The tick of the previously handled event, for accumulating busy_time
    last_tick: Tick,

    /// Measured statistics, to compare against the formulas from (a) and (b)
    stats: Stats,
}

/// Directly measured quantities from a simulation run
#[derive(Debug, Default)]
struct Stats {
    /// busy_time[i] = T_i, the total time exactly i servers were busy
    busy_time: [u64; 3],

    /// Number of repairs that finished within the simulation
    completions: u64,

    /// Total time computers spent in the fix queue, and how many queue visits that covers
    /// (a wait is counted when its repair starts)
    wait_total: u64,
    waits: u64,
}

impl Simulation {
    pub fn new(args: &Args, seed: u64) -> Self {
        // the RNG
        let mut rng = SmallRng::seed_from_u64(seed);

        let mut events = BinaryHeap::new();

        // our computers, each with its first break already scheduled
        let computers: Vec<Option<Computer>> = (0..args.machines)
            .map(|id| {
                let mut computer = Computer::new(id, args, &mut rng);
                let at = Tick(computer.sample_break());
                events.push(Reverse((at, Event::Break(id))));
                Some(computer.set_broken_at(at))
            })
            .collect();

        Self {
            servers: Default::default(),
            fix_queue: Vec::new(),
            computers,
            duration: args.duration,
            events,
            last_tick: Tick(0),
            stats: Stats::default(),
        }
    }

    fn busy_servers(&self) -> usize {
        self.servers.iter().filter(|s| !s.is_idle()).count()
    }

    /// Credits the time since the last event to the current number of busy servers
    fn advance_to(&mut self, tick: Tick) {
        let busy = self.busy_servers();
        self.stats.busy_time[busy] += *tick - *self.last_tick;
        self.last_tick = tick;
    }

    /// Moves the head of the fix queue onto any idle server and schedules when it finishes
    fn assign_idle_servers(&mut self, tick: Tick) {
        for (idx, server) in self.servers.iter_mut().enumerate() {
            if !server.is_idle() {
                continue;
            }
            let Some(mut computer) = self.fix_queue.pop() else {
                break;
            };

            // record how long this computer sat in the queue
            let broke_at = computer
                .broken_until()
                .expect("a queued computer must have a break time");
            let wait = *tick - *broke_at;
            self.stats.wait_total += wait;
            self.stats.waits += 1;

            let done = Tick(*tick + computer.sample_fix());
            server.current_item = Some(computer.set_taken_at(tick).set_fixed_at(done));
            self.events.push(Reverse((done, Event::RepairDone(idx))));
        }
    }

    /// Step handles a single event
    fn step(&mut self, tick: Tick, event: Event) {
        match event {
            // a computer breaks: move it from the computers list to the fix queue
            Event::Break(id) => {
                let computer = self.computers[id as usize]
                    .take()
                    .expect("a broken computer must be in the computers list");
                self.fix_queue.push(computer);
            }

            // a repair finishes: schedule the computer's next break and return it to the list
            Event::RepairDone(idx) => {
                let mut computer = self.servers[idx]
                    .finish_computer()
                    .expect("a finishing server must hold a computer");
                self.stats.completions += 1;
                let id = computer.id();
                let at = Tick(*tick + computer.sample_break());
                self.events.push(Reverse((at, Event::Break(id))));
                self.computers[id as usize] = Some(computer.set_broken_at(at));
            }
        }

        // is either server not busy? if so, move the head of the queue to that server
        self.assign_idle_servers(tick);
    }

    /// Runs the simulation until `duration`, returning the measured statistics
    fn run(mut self) -> Stats {
        while let Some(Reverse((tick, event))) = self.events.pop() {
            if *tick > self.duration {
                break;
            }
            self.advance_to(tick);
            self.step(tick, event);
        }

        // credit the time between the last event and the end of the simulation
        self.advance_to(Tick(self.duration));
        self.stats
    }
}

fn main() {
    let args = Args::parse();
    let seed = args.seed.unwrap_or_else(random);
    let stats = Simulation::new(&args, seed).run();
    let [t0, t1, t2] = stats.busy_time;

    let n = args.machines as f64;
    let p = args.repair_time as f64;
    let r = args.interbreak_time as f64;
    let t = args.duration as f64;

    // (a) throughput X = (T_1 + 2 T_2) / (P T)
    let throughput = (t1 as f64 + 2.0 * t2 as f64) / (p * t);
    // (b) queueing time Q = N / X - P - R
    let queue_time = n / throughput - p - r;

    // the same quantities, measured directly
    let measured_throughput = stats.completions as f64 / t;
    let measured_queue_time = stats.wait_total as f64 / stats.waits as f64;

    println!("seed = {seed}");
    println!("N = {}, P = {p}, R = {r}, T = {t}", args.machines);
    println!();
    println!("T0 = {t0} ({:.4})", t0 as f64 / t);
    println!("T1 = {t1} ({:.4})", t1 as f64 / t);
    println!("T2 = {t2} ({:.4})", t2 as f64 / t);
    println!();
    println!("{:<20}{:>16}{:>16}", "", "formula", "measured");
    println!(
        "{:<20}{:>16.6}{:>16.6}",
        "X (repairs/min)", throughput, measured_throughput
    );
    println!(
        "{:<20}{:>16.2}{:>16.2}",
        "Q (min)", queue_time, measured_queue_time
    );
}
