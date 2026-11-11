# Monte Carlo Simulation

Part of an assignment for a grad level networking class. This simulation is what
I used to validate my analytical model.

# Usage

```sh
cargo run --release -- [OPTIONS]
```

The program models a closed machine shop: `N` machines each run for a while, break,
and wait for one of **2 repair servers**. Time is discrete, and one tick is one minute.
Both the time-to-failure and the repair time are geometric, with the means set by
`-r` and `-p`.

## Arguments

| Flag | Long form           | Symbol | Default             | Meaning                                                                      |
| ---- | ------------------- | ------ | ------------------- | ---------------------------------------------------------------------------- |
| `-n` | `--machines`        | N      | `10`                | Number of machines in the system.                                            |
| `-p` | `--repair-time`     | P      | `480` (8 hours)     | Mean repair time per machine, in minutes.                                    |
| `-r` | `--interbreak-time` | R      | `20160` (2 weeks)   | Mean time from a finished repair until that machine fails again, in minutes. |
| `-t` | `--duration`        | T      | `518400` (360 days) | Total simulated time, in minutes.                                            |
| `-s` | `--seed`            |        | random              | RNG seed. The seed is always printed, so you can reproduce any run.          |
| `-h` | `--help`            |        |                     | Print help.                                                                  |
| `-V` | `--version`         |        |                     | Print version.                                                               |

## Example

In this example, we have 10 machines, each fix takes on average 8 hours, and the
time between breaks is 1 week. The simulation is run for 200,000 years, and has seed 42.
A long run is needed because the formula for Q subtracts two large, nearly equal
numbers (N/X and P + R). Even a small error in X causes a large error in Q, and
over short runs it can come out negative.

```sh
cargo run --release -- -n 10 -p 480 -r 10080 -t 103680000000 -s 42
```

The output is as follows:

```
seed = 42
N = 10, P = 480, R = 10080, T = 103680000000

T0 = 64741589689 (0.6244)
T1 = 30827426775 (0.2973)
T2 = 8110983536 (0.0782)

                             formula        measured
X (repairs/min)             0.000945        0.000945
Q (min)                        17.48           18.19
```
