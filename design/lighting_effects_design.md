# Lighting Effects and Chasers System Design

## Overview

This document outlines the design for a comprehensive effects and chasers system for mtrack, inspired by professional lighting consoles like grandMA. The system will provide powerful, flexible lighting effects that can be easily programmed and executed.

## Core Concepts

### Effects
Effects are dynamic lighting patterns that modify fixture parameters over time. They can be:
- **Static**: Fixed parameter values (e.g., "Red at 100%")
- **Dynamic**: Animated parameter changes (e.g., "Color cycle through rainbow")
- **Spatial**: Effects that move across fixtures (e.g., "Chase from left to right")

### Chasers
Chasers are sequences of effects that play in order, creating complex lighting sequences. They can:
- Loop continuously
- Play once and stop
- Reverse direction
- Have variable timing between steps

## System Architecture

### 1. Effect Engine

```rust
// Core effect types
#[derive(Debug, Clone)]
pub enum EffectType {
    // Static effects
    Static { 
        parameters: HashMap<String, f64>,
        duration: Option<Duration> 
    },
    
    // Dynamic effects
    ColorCycle {
        colors: Vec<Color>,
        speed: f64, // cycles per second
        direction: CycleDirection,
    },
    
    Strobe {
        frequency: f64, // Hz
        intensity: f64, // 0.0 to 1.0
        duration: Option<Duration>,
    },
    
    Dimmer {
        start_level: f64,
        end_level: f64,
        duration: Duration,
        curve: DimmerCurve,
    },
    
    // Spatial effects
    Chase {
        pattern: ChasePattern,
        speed: f64,
        direction: ChaseDirection,
    },
    
    // Complex effects
    Rainbow {
        speed: f64,
        saturation: f64,
        brightness: f64,
    },
    
    Pulse {
        base_level: f64,
        pulse_amplitude: f64,
        frequency: f64,
        duration: Option<Duration>,
    },
}

#[derive(Debug, Clone)]
pub enum CycleDirection {
    Forward,
    Backward,
    PingPong,
}

#[derive(Debug, Clone)]
pub enum ChasePattern {
    Linear,
    Snake,
    Random,
    Custom(Vec<usize>), // Custom fixture order
}

#[derive(Debug, Clone)]
pub enum ChaseDirection {
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
    Clockwise,
    CounterClockwise,
}

#[derive(Debug, Clone)]
pub enum DimmerCurve {
    Linear,
    Exponential,
    Logarithmic,
    Sine,
    Cosine,
    Custom(Vec<f64>), // Custom curve points
}
```

### 2. Effect Instance

```rust
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub id: String,
    pub effect_type: EffectType,
    pub target_fixtures: Vec<String>, // Fixture names or group names
    pub priority: u8, // Higher priority overrides lower
    pub start_time: Option<Instant>,
    pub duration: Option<Duration>,
    pub fade_in: Option<Duration>,
    pub fade_out: Option<Duration>,
    pub enabled: bool,
}

impl EffectInstance {
    pub fn new(id: String, effect_type: EffectType, target_fixtures: Vec<String>) -> Self {
        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            start_time: None,
            duration: None,
            fade_in: None,
            fade_out: None,
            enabled: true,
        }
    }
    
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
    
    pub fn with_timing(mut self, start_time: Option<Instant>, duration: Option<Duration>) -> Self {
        self.start_time = start_time;
        self.duration = duration;
        self
    }
    
    pub fn with_fades(mut self, fade_in: Option<Duration>, fade_out: Option<Duration>) -> Self {
        self.fade_in = fade_in;
        self.fade_out = fade_out;
        self
    }
}
```

### 3. Chaser System

```rust
#[derive(Debug, Clone)]
pub struct ChaserStep {
    pub effect: EffectInstance,
    pub hold_time: Duration,
    pub transition_time: Duration,
    pub transition_type: TransitionType,
}

#[derive(Debug, Clone)]
pub enum TransitionType {
    Snap, // Instant change
    Fade, // Smooth transition
    Crossfade, // Overlap with previous step
    Wipe, // Sequential transition
}

#[derive(Debug, Clone)]
pub struct Chaser {
    pub id: String,
    pub name: String,
    pub steps: Vec<ChaserStep>,
    pub loop_mode: LoopMode,
    pub direction: ChaserDirection,
    pub speed_multiplier: f64,
    pub enabled: bool,
}

#[derive(Debug, Clone)]
pub enum LoopMode {
    Once, // Play once and stop
    Loop, // Repeat indefinitely
    PingPong, // Forward then backward
    Random, // Random step order
}

#[derive(Debug, Clone)]
pub enum ChaserDirection {
    Forward,
    Backward,
    Random,
}

impl Chaser {
    pub fn new(id: String, name: String) -> Self {
        Self {
            id,
            name,
            steps: Vec::new(),
            loop_mode: LoopMode::Loop,
            direction: ChaserDirection::Forward,
            speed_multiplier: 1.0,
            enabled: true,
        }
    }
    
    pub fn add_step(mut self, step: ChaserStep) -> Self {
        self.steps.push(step);
        self
    }
    
    pub fn with_loop_mode(mut self, loop_mode: LoopMode) -> Self {
        self.loop_mode = loop_mode;
        self
    }
    
    pub fn with_speed(mut self, speed_multiplier: f64) -> Self {
        self.speed_multiplier = speed_multiplier;
        self
    }
}
```

### 4. Effect Engine

```rust
pub struct EffectEngine {
    active_effects: HashMap<String, EffectInstance>,
    active_chasers: HashMap<String, ChaserInstance>,
    fixture_registry: HashMap<String, FixtureInfo>,
    current_time: Instant,
}

#[derive(Debug, Clone)]
pub struct FixtureInfo {
    pub name: String,
    pub universe: u16,
    pub address: u16,
    pub fixture_type: String,
    pub channels: HashMap<String, u16>,
}

#[derive(Debug, Clone)]
pub struct ChaserInstance {
    pub chaser: Chaser,
    pub current_step: usize,
    pub step_start_time: Instant,
    pub is_running: bool,
    pub direction: ChaserDirection,
}

impl EffectEngine {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            active_chasers: HashMap::new(),
            fixture_registry: HashMap::new(),
            current_time: Instant::now(),
        }
    }
    
    pub fn register_fixture(&mut self, fixture: FixtureInfo) {
        self.fixture_registry.insert(fixture.name.clone(), fixture);
    }
    
    pub fn start_effect(&mut self, effect: EffectInstance) -> Result<(), EffectError> {
        // Validate effect
        self.validate_effect(&effect)?;
        
        // Stop any conflicting effects
        self.stop_conflicting_effects(&effect);
        
        // Start the effect
        self.active_effects.insert(effect.id.clone(), effect);
        Ok(())
    }
    
    pub fn stop_effect(&mut self, effect_id: &str) {
        self.active_effects.remove(effect_id);
    }
    
    pub fn start_chaser(&mut self, chaser: Chaser) -> Result<(), EffectError> {
        let instance = ChaserInstance {
            chaser: chaser.clone(),
            current_step: 0,
            step_start_time: self.current_time,
            is_running: true,
            direction: chaser.direction.clone(),
        };
        
        self.active_chasers.insert(chaser.id.clone(), instance);
        Ok(())
    }
    
    pub fn stop_chaser(&mut self, chaser_id: &str) {
        self.active_chasers.remove(chaser_id);
    }
    
    pub fn update(&mut self, dt: Duration) -> Result<Vec<DmxCommand>, EffectError> {
        self.current_time += dt;
        let mut commands = Vec::new();
        
        // Update active effects
        for effect in self.active_effects.values() {
            if let Some(commands_for_effect) = self.process_effect(effect)? {
                commands.extend(commands_for_effect);
            }
        }
        
        // Update active chasers
        for chaser_instance in self.active_chasers.values_mut() {
            if let Some(commands_for_chaser) = self.process_chaser(chaser_instance)? {
                commands.extend(commands_for_chaser);
            }
        }
        
        Ok(commands)
    }
    
    fn process_effect(&self, effect: &EffectInstance) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        if !effect.enabled {
            return Ok(None);
        }
        
        // Calculate effect parameters based on current time
        let elapsed = self.current_time.duration_since(effect.start_time.unwrap_or(self.current_time));
        
        match &effect.effect_type {
            EffectType::Static { parameters, .. } => {
                self.apply_static_effect(effect, parameters)
            },
            EffectType::ColorCycle { colors, speed, direction } => {
                self.apply_color_cycle(effect, colors, *speed, direction, elapsed)
            },
            EffectType::Strobe { frequency, intensity, .. } => {
                self.apply_strobe(effect, *frequency, *intensity, elapsed)
            },
            EffectType::Dimmer { start_level, end_level, duration, curve } => {
                self.apply_dimmer(effect, *start_level, *end_level, *duration, curve, elapsed)
            },
            EffectType::Chase { pattern, speed, direction } => {
                self.apply_chase(effect, pattern, *speed, direction, elapsed)
            },
            EffectType::Rainbow { speed, saturation, brightness } => {
                self.apply_rainbow(effect, *speed, *saturation, *brightness, elapsed)
            },
            EffectType::Pulse { base_level, pulse_amplitude, frequency, .. } => {
                self.apply_pulse(effect, *base_level, *pulse_amplitude, *frequency, elapsed)
            },
        }
    }
    
    fn process_chaser(&mut self, chaser_instance: &mut ChaserInstance) -> Result<Option<Vec<DmxCommand>>, EffectError> {
        if !chaser_instance.is_running {
            return Ok(None);
        }
        
        let step_duration = chaser_instance.chaser.steps[chaser_instance.current_step].hold_time;
        let elapsed = self.current_time.duration_since(chaser_instance.step_start_time);
        
        if elapsed >= step_duration {
            // Move to next step
            self.advance_chaser_step(chaser_instance);
        }
        
        // Process current step
        let current_step = &chaser_instance.chaser.steps[chaser_instance.current_step];
        self.process_effect(&current_step.effect)
    }
    
    fn advance_chaser_step(&mut self, chaser_instance: &mut ChaserInstance) {
        match chaser_instance.direction {
            ChaserDirection::Forward => {
                chaser_instance.current_step += 1;
                if chaser_instance.current_step >= chaser_instance.chaser.steps.len() {
                    match chaser_instance.chaser.loop_mode {
                        LoopMode::Once => {
                            chaser_instance.is_running = false;
                            return;
                        },
                        LoopMode::Loop => {
                            chaser_instance.current_step = 0;
                        },
                        LoopMode::PingPong => {
                            chaser_instance.direction = ChaserDirection::Backward;
                            chaser_instance.current_step -= 1;
                        },
                        LoopMode::Random => {
                            chaser_instance.current_step = rand::thread_rng().gen_range(0..chaser_instance.chaser.steps.len());
                        },
                    }
                }
            },
            ChaserDirection::Backward => {
                if chaser_instance.current_step > 0 {
                    chaser_instance.current_step -= 1;
                } else {
                    match chaser_instance.chaser.loop_mode {
                        LoopMode::Once => {
                            chaser_instance.is_running = false;
                            return;
                        },
                        LoopMode::Loop => {
                            chaser_instance.current_step = chaser_instance.chaser.steps.len() - 1;
                        },
                        LoopMode::PingPong => {
                            chaser_instance.direction = ChaserDirection::Forward;
                            chaser_instance.current_step += 1;
                        },
                        LoopMode::Random => {
                            chaser_instance.current_step = rand::thread_rng().gen_range(0..chaser_instance.chaser.steps.len());
                        },
                    }
                }
            },
            ChaserDirection::Random => {
                chaser_instance.current_step = rand::thread_rng().gen_range(0..chaser_instance.chaser.steps.len());
            },
        }
        
        chaser_instance.step_start_time = self.current_time;
    }
}

#[derive(Debug, Clone)]
pub struct DmxCommand {
    pub universe: u16,
    pub channel: u16,
    pub value: u8,
}

#[derive(Debug)]
pub enum EffectError {
    InvalidFixture(String),
    InvalidParameter(String),
    InvalidTiming(String),
    EngineError(String),
}
```

## DSL Extensions

### Effect Definition DSL

```rust
// Example DSL for defining effects
let color_cycle = r#"
effect "Rainbow Cycle" {
    type: color_cycle
    colors: [
        { r: 255, g: 0, b: 0 },    // Red
        { r: 255, g: 127, b: 0 }, // Orange
        { r: 255, g: 255, b: 0 }, // Yellow
        { r: 0, g: 255, b: 0 },   // Green
        { r: 0, g: 0, b: 255 },   // Blue
        { r: 127, g: 0, b: 255 }, // Purple
    ]
    speed: 2.0  // cycles per second
    direction: forward
    duration: 30s
}
"#;

let chase_effect = r#"
effect "Left to Right Chase" {
    type: chase
    pattern: linear
    direction: left_to_right
    speed: 1.5
    fixtures: ["wash1", "wash2", "wash3", "wash4"]
    color: { r: 255, g: 255, b: 255 }
    intensity: 100%
}
"#;
```

### Chaser Definition DSL

```rust
let chaser = r#"
chaser "Club Beat Chaser" {
    loop: true
    direction: forward
    speed: 1.0
    
    step 1 {
        effect: "Red Flash"
        hold: 0.5s
        transition: snap
    }
    
    step 2 {
        effect: "Blue Flash"
        hold: 0.5s
        transition: fade
        transition_time: 0.2s
    }
    
    step 3 {
        effect: "Green Flash"
        hold: 0.5s
        transition: crossfade
        transition_time: 0.3s
    }
}
"#;
```

## Integration with Song System

### Song-Level Effect Triggers

```yaml
# In song definition
lighting:
  effects:
    - trigger: "beat:1.1"  # Beat 1.1
      effect: "Red Flash"
      fixtures: ["wash1", "wash2"]
      duration: 0.5s
    
    - trigger: "beat:2.1"
      effect: "Blue Chase"
      fixtures: ["wash3", "wash4", "wash5"]
      duration: 2.0s
    
    - trigger: "time:1:30.5"  # Absolute time
      effect: "Rainbow Cycle"
      fixtures: ["all_wash"]
      duration: 10.0s

  chasers:
    - trigger: "beat:1.1"
      chaser: "Club Beat Chaser"
      fixtures: ["all_wash"]
      loop: true
      stop_trigger: "beat:32.1"
```

## Performance Considerations

1. **Efficient Updates**: Only recalculate effects that are actually running
2. **Priority System**: Higher priority effects override lower priority ones
3. **Fixture Grouping**: Process fixtures in groups to reduce DMX channel updates
4. **Time-based Optimization**: Use time-based calculations instead of frame-based where possible

## Future Enhancements

1. **Effect Presets**: Pre-defined effect combinations
2. **Effect Layering**: Multiple effects on the same fixtures
3. **Effect Masking**: Selective application of effects to fixture parameters
4. **Real-time Control**: MIDI/OSC control of effect parameters
5. **Effect Recording**: Record and playback of effect sequences
6. **Effect Templates**: Reusable effect definitions across songs

This design provides a comprehensive foundation for professional-grade lighting effects while maintaining the simplicity and power that mtrack users expect.
