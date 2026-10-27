//! Client-array state: what the PROGRAM holds (pointers, enables, sizes, types, strides). The engine owns it - the worker's
//! arrays are enabled only for the length of one draw - so the engine answers the queries about it (`sig::engine_answers`) and
//! never asks the worker.
use super::*;
use alloc::vec;
use heirloom_gl_abi::family::Ty;

const GL_FLOAT: f64 = 5126.0;

impl<U: GuestUi> MacPpc<U> {
    fn array_of(&self, role: Role, unit: u8) -> Option<&Descriptor> {
        self.gl.arrays.get(&(role as u8, unit))
    }

    /// The values the client-array query `pname` answers (`sig::engine_answers`), by GL 1.2 Table 6.7: the initial size is 4 (3 for
    /// the normal), the initial type GL_FLOAT, the initial stride 0, every array disabled.
    pub(super) fn gl_client_values(&self, pname: u32) -> Option<Vec<f64>> {
        let unit = self.gl.client_unit;
        let en = |role: Role, u: u8| f64::from(u8::from(self.gl.enabled.contains(&(role as u8, u))));
        let size = |role: Role, u: u8| f64::from(self.array_of(role, u).map_or(4, |d| d.size));
        let ty = |role: Role, u: u8| self.array_of(role, u).map_or(GL_FLOAT, |d| f64::from(d.ty as u32));
        let stride = |role: Role, u: u8| self.array_of(role, u).map_or(0.0, |d| f64::from(d.stride));
        let v = match pname {
            0x8074 => en(Role::Vertex, 0),
            0x8075 => en(Role::Normal, 0),
            0x8076 => en(Role::Color, 0),
            0x8077 => f64::from(u8::from(self.gl.index_array.is_some_and(|a| a.enabled))),
            0x8078 => en(Role::TexCoord, unit),
            0x8079 => f64::from(u8::from(self.gl.edge_array.is_some_and(|a| a.enabled))),
            0x807A => size(Role::Vertex, 0),
            0x807B => ty(Role::Vertex, 0),
            0x807C => stride(Role::Vertex, 0),
            0x807E => ty(Role::Normal, 0),
            0x807F => stride(Role::Normal, 0),
            0x8081 => size(Role::Color, 0),
            0x8082 => ty(Role::Color, 0),
            0x8083 => stride(Role::Color, 0),
            0x8085 => self.gl.index_array.map_or(GL_FLOAT, |a| f64::from(a.ty)),
            0x8086 => self.gl.index_array.map_or(0.0, |a| f64::from(a.stride)),
            0x8088 => size(Role::TexCoord, unit),
            0x8089 => ty(Role::TexCoord, unit),
            0x808A => stride(Role::TexCoord, unit),
            0x808C => self.gl.edge_array.map_or(0.0, |a| f64::from(a.stride)),
            _ => return None,
        };
        Some(vec![v])
    }

    /// The reply words of an engine-answered query asked as type `t`.
    pub(super) fn gl_client_words(&self, pname: u32, t: Ty) -> Option<Vec<u64>> {
        let vals = self.gl_client_values(pname)?;
        Some(vals.into_iter().map(|v| match t {
            Ty::Bool => u64::from(v != 0.0),
            Ty::Float => u64::from((v as f32).to_bits()),
            Ty::Double => v.to_bits(),
            _ => u64::from(v as i64 as u32),
        }).collect())
    }
}
