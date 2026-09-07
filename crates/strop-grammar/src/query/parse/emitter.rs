use super::*;

// ------------------------------------------------------------------ emit

pub(super) struct Emitter {
    pub(super) insts: Vec<Inst>,
    pub(super) n_slots: usize,
    pub(super) n_loops: usize,
    pub(super) fold: bool,
}

impl Emitter {
    pub(super) fn node(&mut self, n: &Node) -> Result<(), QueryError> {
        if self.insts.len() > MAX_INSTS {
            return Err(QueryError::TooComplex);
        }
        match n {
            Node::Seq(items) => {
                for i in items {
                    self.node(i)?;
                }
            }
            Node::Alt(branches) => {
                let mut jumps = Vec::new();
                for (i, b) in branches.iter().enumerate() {
                    if i + 1 == branches.len() {
                        self.node(b)?;
                    } else {
                        let split = self.insts.len() as u32;
                        self.insts.push(Inst::Split { prefer: 0, alt: 0 });
                        let body = self.insts.len() as u32;
                        self.node(b)?;
                        jumps.push(self.insts.len() as u32);
                        self.insts.push(Inst::Jmp(0));
                        let alt = self.insts.len() as u32;
                        self.insts[split as usize] = Inst::Split { prefer: body, alt };
                    }
                }
                let out = self.insts.len() as u32;
                for j in jumps {
                    self.insts[j as usize] = Inst::Jmp(out);
                }
            }
            Node::Rep {
                node,
                min,
                max,
                greedy,
            } => self.rep(node, *min, *max, *greedy)?,
            Node::Group { idx, node } => match idx {
                Some(g) => {
                    let s = 2 * g;
                    self.insts.push(Inst::Save(s));
                    self.node(node)?;
                    self.insts.push(Inst::Save(s + 1));
                }
                None => self.node(node)?,
            },
            Node::PrefixOpt(items) => {
                // a(b(c)?)? — each item tried in order, exit skips the rest
                let mut splits = Vec::new();
                for item in items {
                    let split = self.insts.len() as u32;
                    self.insts.push(Inst::Split { prefer: 0, alt: 0 });
                    splits.push(split);
                    let body = self.insts.len() as u32;
                    self.node(item)?;
                    let _ = body;
                }
                let out = self.insts.len() as u32;
                for split in splits {
                    let body = split + 1;
                    self.insts[split as usize] = Inst::Split {
                        prefer: body,
                        alt: out,
                    };
                }
            }
            Node::Literal(bytes) => {
                self.insts.push(Inst::Consume(Class::Str {
                    bytes: bytes.clone(),
                    fold: self.fold,
                }));
            }
            Node::Set(s) => {
                let mut s = s.clone();
                s.fold = self.fold;
                self.insts.push(Inst::Consume(Class::Set(s)));
            }
            Node::Any { nl } => self.insts.push(Inst::Consume(Class::Any { nl: *nl })),
            Node::Break => self.insts.push(Inst::Consume(Class::Break)),
            Node::Backref(g) => {
                if 2 * g + 1 < self.n_slots {
                    self.insts.push(Inst::Backref { group: *g });
                } else {
                    // a backref to a group that doesn't exist never matches
                    self.insts.push(Inst::Fail);
                }
            }
            Node::LineStart => self.insts.push(Inst::LineStart),
            Node::LineEnd => self.insts.push(Inst::LineEnd),
            Node::BufStart => self.insts.push(Inst::BufStart),
            Node::BufEnd => self.insts.push(Inst::BufEnd),
            Node::WordStart => self.insts.push(Inst::WordStart),
            Node::WordEnd => self.insts.push(Inst::WordEnd),
            Node::SetStart => self.insts.push(Inst::Save(0)),
            Node::SetEnd => self.insts.push(Inst::Save(1)),
        }
        Ok(())
    }

    fn rep(
        &mut self,
        node: &Node,
        min: u32,
        max: Option<u32>,
        greedy: bool,
    ) -> Result<(), QueryError> {
        for _ in 0..min {
            self.node(node)?;
        }
        match max {
            None => {
                // unbounded loop with a zero-width guard at the head
                let id = self.n_loops;
                self.n_loops += 1;
                let guard = self.insts.len() as u32;
                self.insts.push(Inst::Guard(id));
                let split = self.insts.len() as u32;
                self.insts.push(Inst::Split { prefer: 0, alt: 0 });
                let body = self.insts.len() as u32;
                self.node(node)?;
                self.insts.push(Inst::Jmp(guard));
                let out = self.insts.len() as u32;
                self.insts[split as usize] = if greedy {
                    Inst::Split {
                        prefer: body,
                        alt: out,
                    }
                } else {
                    Inst::Split {
                        prefer: out,
                        alt: body,
                    }
                };
            }
            Some(max) => {
                // bounded: (max - min) nested optionals
                let extra = max.saturating_sub(min);
                let mut splits = Vec::new();
                for _ in 0..extra {
                    let split = self.insts.len() as u32;
                    self.insts.push(Inst::Split { prefer: 0, alt: 0 });
                    splits.push(split);
                    self.node(node)?;
                }
                let out = self.insts.len() as u32;
                for split in splits {
                    let body = split + 1;
                    self.insts[split as usize] = if greedy {
                        Inst::Split {
                            prefer: body,
                            alt: out,
                        }
                    } else {
                        Inst::Split {
                            prefer: out,
                            alt: body,
                        }
                    };
                }
            }
        }
        Ok(())
    }
}
