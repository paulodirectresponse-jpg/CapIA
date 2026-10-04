//! Propriedades animáveis e keyframes (docs/TIMELINE_ENGINE.md §3 "Keyframes").
//!
//! * O **tempo** de keyframe é inteiro ([`Ticks`]) e relativo ao **conteúdo** do clip (tempo de
//!   origem para mídia). Os **valores** são `f64` (parâmetros não temporais — regra 1 do §1.3).
//! * A avaliação é uma função pura e determinística: só `+ − × ÷` e comparações em `f64` (sem
//!   funções transcendentais), então preview, export e WASM concordam bit a bit.
//! * D-S7-6: recortar (`trim`) **nunca** apaga keyframes; `split` calcula o valor interpolado no ponto
//!   de divisão e cria o estado de fronteira nas duas metades ([`Animatable::split_at`]).

use capia_time::Ticks;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Faixa válida de uma propriedade. O core **rejeita** valores fora dela (D-S7-7); a UI pode fazer
/// clamp preventivo, mas o core nunca depende disso.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PropertySpec {
    pub name: &'static str,
    pub min: f64,
    pub max: f64,
    pub default: f64,
}

impl PropertySpec {
    pub fn contains(&self, v: f64) -> bool {
        v.is_finite() && v >= self.min && v <= self.max
    }

    pub fn clamp(&self, v: f64) -> f64 {
        v.clamp(self.min, self.max)
    }
}

const PROPERTY_SPECS: [PropertySpec; 8] = [
    PropertySpec {
        name: "opacity",
        min: 0.0,
        max: 1.0,
        default: 1.0,
    },
    PropertySpec {
        name: "scale",
        min: 0.0,
        max: 100.0,
        default: 1.0,
    },
    PropertySpec {
        name: "position_x",
        min: -1_000_000.0,
        max: 1_000_000.0,
        default: 0.0,
    },
    PropertySpec {
        name: "position_y",
        min: -1_000_000.0,
        max: 1_000_000.0,
        default: 0.0,
    },
    PropertySpec {
        name: "rotation",
        min: -36_000.0,
        max: 36_000.0,
        default: 0.0,
    },
    PropertySpec {
        name: "volume_db",
        min: -120.0,
        max: 24.0,
        default: 0.0,
    },
    // Fades de clip (segundos): rampa linear de opacidade (visual) e de ganho (áudio) nas bordas.
    PropertySpec {
        name: "fade_in",
        min: 0.0,
        max: 60.0,
        default: 0.0,
    },
    PropertySpec {
        name: "fade_out",
        min: 0.0,
        max: 60.0,
        default: 0.0,
    },
];

/// Registro de propriedades conhecidas.
pub fn property_spec(name: &str) -> Option<&'static PropertySpec> {
    PROPERTY_SPECS.iter().find(|s| s.name == name)
}

/// Todas as propriedades conhecidas (para documentação/schema).
pub fn property_specs() -> &'static [PropertySpec] {
    &PROPERTY_SPECS
}

/// Interpolação do **segmento que começa** no keyframe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Interp {
    /// Mantém o valor do keyframe até o próximo.
    Hold,
    #[default]
    Linear,
    /// `cubic-bezier(x1, y1, x2, y2)` normalizado (como CSS). `x1, x2 ∈ [0, 1]`; `y` pode ultrapassar
    /// 0..1 (overshoot), mas o valor final é sempre limitado à faixa da propriedade.
    Bezier { x1: f64, y1: f64, x2: f64, y2: f64 },
}

impl Interp {
    /// `Err` com o motivo se os parâmetros forem inválidos.
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Self::Bezier { x1, y1, x2, y2 } = *self {
            if ![x1, y1, x2, y2].iter().all(|v| v.is_finite()) {
                return Err("bezier parameters must be finite");
            }
            if !(0.0..=1.0).contains(&x1) || !(0.0..=1.0).contains(&x2) {
                return Err("bezier x1/x2 must be within [0, 1]");
            }
            if y1.abs() > 8.0 || y2.abs() > 8.0 {
                return Err("bezier y1/y2 must be within [-8, 8]");
            }
        }
        Ok(())
    }

    /// Fração de progresso do valor para a fração de tempo `u ∈ [0, 1]`.
    pub fn ease(&self, u: f64) -> f64 {
        let u = u.clamp(0.0, 1.0);
        match *self {
            Self::Hold => 0.0,
            Self::Linear => u,
            Self::Bezier { x1, y1, x2, y2 } => {
                let s = solve_bezier_x(x1, x2, u);
                bezier_coord(y1, y2, s)
            }
        }
    }
}

/// Coordenada de um Bézier cúbico com extremos 0 e 1: `3(1−s)²s·a + 3(1−s)s²·b + s³`.
fn bezier_coord(a: f64, b: f64, s: f64) -> f64 {
    let inv = 1.0 - s;
    3.0 * inv * inv * s * a + 3.0 * inv * s * s * b + s * s * s
}

/// Parâmetro `s` tal que `x(s) = u` (x é monótono para `x1, x2 ∈ [0, 1]`). Bisseção: 64 passos.
fn solve_bezier_x(x1: f64, x2: f64, u: f64) -> f64 {
    let (mut lo, mut hi) = (0.0_f64, 1.0_f64);
    for _ in 0..64 {
        let mid = 0.5 * (lo + hi);
        if bezier_coord(x1, x2, mid) < u {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

/// Keyframe: `time` em ticks de **conteúdo** do clip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Keyframe {
    pub time: Ticks,
    pub value: f64,
    #[serde(default)]
    pub interp: Interp,
}

/// Valor estático ou animado. Em `Animated` os keyframes são **estritamente crescentes** em `time`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Animatable {
    Static(f64),
    Animated(Vec<Keyframe>),
}

/// Conjunto de propriedades de um clip, por nome (ver [`property_spec`]).
pub type PropertySet = BTreeMap<String, Animatable>;

/// Resultado de [`Animatable::split_at`].
#[derive(Clone, Debug, PartialEq)]
pub struct SplitAnimatable {
    pub left: Animatable,
    pub right: Animatable,
}

/// Divide a curva de um segmento no ponto de fração de tempo `s ∈ (0, 1)`. Devolve
/// `(interp_esquerda, interp_direita, fração_de_valor)` tais que avaliar cada metade, com tempo
/// renormalizado, reproduz exatamente a curva original (de Casteljau).
fn split_segment(interp: Interp, s: f64) -> (Interp, Interp, f64) {
    match interp {
        Interp::Hold => (Interp::Hold, Interp::Hold, 0.0),
        Interp::Linear => (Interp::Linear, Interp::Linear, s),
        Interp::Bezier { x1, y1, x2, y2 } => {
            let u = solve_bezier_x(x1, x2, s);
            let lerp = |a: (f64, f64), b: (f64, f64), t: f64| {
                (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
            };
            let (p0, p1, p2, p3) = ((0.0, 0.0), (x1, y1), (x2, y2), (1.0, 1.0));
            let (a, b, c) = (lerp(p0, p1, u), lerp(p1, p2, u), lerp(p2, p3, u));
            let (d, e) = (lerp(a, b, u), lerp(b, c, u));
            let m = lerp(d, e, u); // ponto de divisão: (s, e_s)
            let e_s = m.1;
            const EPS: f64 = 1e-9;
            let left = if e_s.abs() < EPS || s < EPS {
                Interp::Linear
            } else {
                Interp::Bezier {
                    x1: a.0 / s,
                    y1: a.1 / e_s,
                    x2: d.0 / s,
                    y2: d.1 / e_s,
                }
            };
            let right = if (1.0 - e_s).abs() < EPS || (1.0 - s) < EPS {
                Interp::Linear
            } else {
                Interp::Bezier {
                    x1: (e.0 - s) / (1.0 - s),
                    y1: (e.1 - e_s) / (1.0 - e_s),
                    x2: (c.0 - s) / (1.0 - s),
                    y2: (c.1 - e_s) / (1.0 - e_s),
                }
            };
            (left, right, e_s)
        }
    }
}

impl Animatable {
    /// Keyframes (vazio para `Static`).
    pub fn keyframes(&self) -> &[Keyframe] {
        match self {
            Self::Static(_) => &[],
            Self::Animated(k) => k,
        }
    }

    /// Valor no tempo de conteúdo `t`: antes do primeiro / depois do último mantém o valor da
    /// ponta; entre dois keyframes usa a interpolação do keyframe anterior. Limitado à faixa.
    pub fn eval(&self, t: Ticks, spec: &PropertySpec) -> f64 {
        let v = match self {
            Self::Static(v) => *v,
            Self::Animated(kfs) => eval_keyframes(kfs, t),
        };
        spec.clamp(v)
    }

    /// Insere o keyframe; **substitui** se já houver um no mesmo tempo. Devolve o substituído.
    pub fn set_keyframe(&mut self, kf: Keyframe) -> Option<Keyframe> {
        match self {
            Self::Static(_) => {
                *self = Self::Animated(vec![kf]);
                None
            }
            Self::Animated(kfs) => match kfs.binary_search_by(|k| k.time.cmp(&kf.time)) {
                Ok(i) => Some(core::mem::replace(&mut kfs[i], kf)),
                Err(i) => {
                    kfs.insert(i, kf);
                    None
                }
            },
        }
    }

    /// Remove o keyframe em `time`. Ao remover o último, a propriedade vira `Static` com o valor
    /// dele (a aparência não muda). `None` se não existe.
    pub fn remove_keyframe(&mut self, time: Ticks) -> Option<Keyframe> {
        let Self::Animated(kfs) = self else {
            return None;
        };
        let i = kfs.binary_search_by(|k| k.time.cmp(&time)).ok()?;
        let removed = kfs.remove(i);
        if kfs.is_empty() {
            *self = Self::Static(removed.value);
        }
        Some(removed)
    }

    /// Divide em `boundary` (tempo de conteúdo) — D-S7-6. Se `rebase_right`, os tempos da metade
    /// direita são deslocados por `-boundary` (clips sem mídia de origem: tempo local recomeça em 0).
    ///
    /// * antes do primeiro keyframe: esquerda vira `Static(primeiro valor)`; direita inalterada;
    /// * depois do último: direita vira `Static(último valor)`; esquerda inalterada;
    /// * sobre um keyframe: ele existe nas duas metades (sem duplicar);
    /// * dentro de um segmento: as duas metades ganham um keyframe de fronteira com o valor
    ///   interpolado e os trechos de curva são divididos com exatidão (inclusive Bézier).
    pub fn split_at(&self, boundary: Ticks, rebase_right: bool) -> SplitAnimatable {
        let shift = |kfs: Vec<Keyframe>| -> Animatable {
            Animatable::Animated(if rebase_right {
                kfs.into_iter()
                    .map(|k| Keyframe {
                        time: Ticks(k.time.0 - boundary.0),
                        ..k
                    })
                    .collect()
            } else {
                kfs
            })
        };
        let kfs = match self {
            Self::Static(v) => {
                return SplitAnimatable {
                    left: Self::Static(*v),
                    right: Self::Static(*v),
                };
            }
            Self::Animated(kfs) if kfs.is_empty() => {
                return SplitAnimatable {
                    left: self.clone(),
                    right: self.clone(),
                };
            }
            Self::Animated(kfs) => kfs,
        };
        let (first, last) = (kfs[0], kfs[kfs.len() - 1]);
        if boundary <= first.time {
            return SplitAnimatable {
                left: Self::Static(first.value),
                right: shift(kfs.clone()),
            };
        }
        if boundary >= last.time {
            return SplitAnimatable {
                left: Self::Animated(kfs.clone()),
                right: Self::Static(last.value),
            };
        }
        match kfs.binary_search_by(|k| k.time.cmp(&boundary)) {
            Ok(i) => SplitAnimatable {
                left: Self::Animated(kfs[..=i].to_vec()),
                right: shift(kfs[i..].to_vec()),
            },
            Err(i) => {
                // kfs[i-1].time < boundary < kfs[i].time
                let (a, b) = (kfs[i - 1], kfs[i]);
                let s = (boundary.0 - a.time.0) as f64 / (b.time.0 - a.time.0) as f64;
                let (left_interp, right_interp, e_s) = split_segment(a.interp, s);
                let value = a.value + (b.value - a.value) * e_s;
                let mut left: Vec<Keyframe> = kfs[..i].to_vec();
                if let Some(prev) = left.last_mut() {
                    prev.interp = left_interp;
                }
                left.push(Keyframe {
                    time: boundary,
                    value,
                    interp: Interp::Linear,
                });
                let mut right = vec![Keyframe {
                    time: boundary,
                    value,
                    interp: right_interp,
                }];
                right.extend_from_slice(&kfs[i..]);
                SplitAnimatable {
                    left: Self::Animated(left),
                    right: shift(right),
                }
            }
        }
    }

    /// Estrutura válida: tempos estritamente crescentes, valores finitos, interps válidas.
    pub fn check_structure(&self) -> Result<(), &'static str> {
        match self {
            Self::Static(v) if !v.is_finite() => Err("static value must be finite"),
            Self::Static(_) => Ok(()),
            Self::Animated(kfs) => {
                if kfs.is_empty() {
                    return Err("animated property has no keyframes");
                }
                for pair in kfs.windows(2) {
                    if pair[0].time >= pair[1].time {
                        return Err("keyframe times must be strictly increasing");
                    }
                }
                for k in kfs {
                    if !k.value.is_finite() {
                        return Err("keyframe value must be finite");
                    }
                    k.interp.validate()?;
                }
                Ok(())
            }
        }
    }
}

fn eval_keyframes(kfs: &[Keyframe], t: Ticks) -> f64 {
    let (Some(first), Some(last)) = (kfs.first(), kfs.last()) else {
        return 0.0;
    };
    if t <= first.time {
        return first.value;
    }
    if t >= last.time {
        return last.value;
    }
    let i = match kfs.binary_search_by(|k| k.time.cmp(&t)) {
        Ok(i) => return kfs[i].value,
        Err(i) => i, // kfs[i-1].time < t < kfs[i].time
    };
    let (a, b) = (kfs[i - 1], kfs[i]);
    let u = (t.0 - a.time.0) as f64 / (b.time.0 - a.time.0) as f64;
    a.value + (b.value - a.value) * a.interp.ease(u)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn kf(t: i64, v: f64) -> Keyframe {
        Keyframe {
            time: Ticks(t),
            value: v,
            interp: Interp::Linear,
        }
    }

    fn opacity() -> &'static PropertySpec {
        property_spec("opacity").unwrap()
    }

    #[test]
    fn linear_hold_and_ends() {
        let a = Animatable::Animated(vec![kf(10, 0.2), kf(20, 0.8)]);
        let at = |t| a.eval(Ticks(t), opacity());
        assert_eq!(at(0), 0.2);
        assert_eq!(at(10), 0.2);
        assert!((at(15) - 0.5).abs() < 1e-12);
        assert_eq!(at(20), 0.8);
        assert_eq!(at(50), 0.8);
        let h = Animatable::Animated(vec![
            Keyframe {
                interp: Interp::Hold,
                ..kf(0, 0.0)
            },
            kf(30, 1.0),
        ]);
        assert_eq!(h.eval(Ticks(29), opacity()), 0.0);
        assert_eq!(h.eval(Ticks(30), opacity()), 1.0);
    }

    #[test]
    fn eval_clamps_bezier_overshoot_to_the_property_range() {
        let b = Interp::Bezier {
            x1: 0.25,
            y1: 1.6,
            x2: 0.75,
            y2: 1.6,
        };
        let a = Animatable::Animated(vec![
            Keyframe {
                interp: b,
                ..kf(0, 0.0)
            },
            kf(30, 1.0),
        ]);
        let mut peaked = false;
        for t in 0..=30 {
            let v = a.eval(Ticks(t), opacity());
            assert!((0.0..=1.0).contains(&v), "t={t} v={v}");
            peaked |= v == 1.0;
        }
        assert!(peaked, "the curve really overshoots and gets clamped");
    }

    #[test]
    fn set_and_remove_keyframes_keep_order_and_static_fallback() {
        let mut a = Animatable::Static(0.3);
        assert!(a.set_keyframe(kf(20, 0.5)).is_none());
        a.set_keyframe(kf(10, 0.1));
        assert_eq!(a.set_keyframe(kf(10, 0.9)).map(|k| k.value), Some(0.1));
        assert_eq!(
            a.keyframes().iter().map(|k| k.time.0).collect::<Vec<_>>(),
            [10, 20]
        );
        assert!(a.remove_keyframe(Ticks(99)).is_none());
        a.remove_keyframe(Ticks(10));
        a.remove_keyframe(Ticks(20));
        assert_eq!(a, Animatable::Static(0.5));
    }

    #[test]
    fn split_inside_adds_interpolated_boundary_to_both_halves() {
        let a = Animatable::Animated(vec![kf(0, 0.0), kf(60, 1.0)]);
        let s = a.split_at(Ticks(30), false);
        assert_eq!(s.left, Animatable::Animated(vec![kf(0, 0.0), kf(30, 0.5)]));
        assert_eq!(
            s.right,
            Animatable::Animated(vec![kf(30, 0.5), kf(60, 1.0)])
        );
    }

    #[test]
    fn split_on_a_keyframe_does_not_duplicate() {
        let a = Animatable::Animated(vec![kf(0, 0.0), kf(30, 0.4), kf(60, 1.0)]);
        let s = a.split_at(Ticks(30), false);
        assert_eq!(s.left, Animatable::Animated(vec![kf(0, 0.0), kf(30, 0.4)]));
        assert_eq!(
            s.right,
            Animatable::Animated(vec![kf(30, 0.4), kf(60, 1.0)])
        );
    }

    #[test]
    fn split_before_first_and_after_last_become_static() {
        let a = Animatable::Animated(vec![kf(40, 0.2), kf(50, 0.8)]);
        let s = a.split_at(Ticks(20), false);
        assert_eq!(s.left, Animatable::Static(0.2));
        assert_eq!(s.right, a);
        let s = a.split_at(Ticks(70), false);
        assert_eq!(s.left, a);
        assert_eq!(s.right, Animatable::Static(0.8));
    }

    #[test]
    fn split_can_rebase_the_right_half_to_local_time() {
        let a = Animatable::Animated(vec![kf(0, 0.0), kf(40, 1.0)]);
        let s = a.split_at(Ticks(20), true);
        assert_eq!(s.left, Animatable::Animated(vec![kf(0, 0.0), kf(20, 0.5)]));
        assert_eq!(s.right, Animatable::Animated(vec![kf(0, 0.5), kf(20, 1.0)]));
    }

    /// Dividir qualquer curva (linear, hold ou Bézier) e avaliar as metades reproduz a original.
    #[test]
    fn split_preserves_the_animation_exactly() {
        let curves = [
            Interp::Linear,
            Interp::Hold,
            Interp::Bezier {
                x1: 0.25,
                y1: 0.1,
                x2: 0.25,
                y2: 1.0,
            },
            Interp::Bezier {
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
            },
            Interp::Bezier {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            },
            Interp::Bezier {
                x1: 0.2,
                y1: -0.4,
                x2: 0.8,
                y2: 1.4,
            },
        ];
        let spec = property_spec("position_x").unwrap();
        for interp in curves {
            let orig = Animatable::Animated(vec![
                Keyframe {
                    time: Ticks(100),
                    value: -50.0,
                    interp,
                },
                Keyframe {
                    time: Ticks(1100),
                    value: 250.0,
                    interp: Interp::Linear,
                },
            ]);
            for boundary in [101, 250, 333, 500, 777, 999, 1099] {
                let s = orig.split_at(Ticks(boundary), false);
                for t in (100..=1100).step_by(7) {
                    let want = orig.eval(Ticks(t), spec);
                    let half = if t <= boundary { &s.left } else { &s.right };
                    let got = half.eval(Ticks(t), spec);
                    assert!(
                        (want - got).abs() < 1e-6,
                        "interp={interp:?} boundary={boundary} t={t} want={want} got={got}"
                    );
                }
                s.left.check_structure().unwrap();
                s.right.check_structure().unwrap();
            }
        }
    }

    #[test]
    fn structure_checks_reject_bad_data() {
        assert!(Animatable::Animated(vec![]).check_structure().is_err());
        assert!(
            Animatable::Animated(vec![kf(5, 0.0), kf(5, 1.0)])
                .check_structure()
                .is_err()
        );
        assert!(
            Animatable::Animated(vec![kf(5, f64::NAN)])
                .check_structure()
                .is_err()
        );
        assert!(Animatable::Static(f64::INFINITY).check_structure().is_err());
        let bad = Interp::Bezier {
            x1: 1.5,
            y1: 0.0,
            x2: 0.5,
            y2: 1.0,
        };
        assert!(bad.validate().is_err());
        assert!(
            Animatable::Animated(vec![Keyframe {
                interp: bad,
                ..kf(0, 0.0)
            }])
            .check_structure()
            .is_err()
        );
    }

    #[test]
    fn json_shape_is_stable() {
        let a = Animatable::Animated(vec![Keyframe {
            time: Ticks(7),
            value: 0.5,
            interp: Interp::Bezier {
                x1: 0.1,
                y1: 0.2,
                x2: 0.3,
                y2: 0.4,
            },
        }]);
        let json = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Animatable>(&json).unwrap(), a);
        let from_min: Keyframe = serde_json::from_str(r#"{"time":1,"value":2.0}"#).unwrap();
        assert_eq!(from_min.interp, Interp::Linear);
    }
}
