use crate::actor::Actor;
use crate::entities::{AgentState, Message, MessageKind};
use crate::graph::TaskGraph;
use crate::ids::{AgentId, ContractId, DecisionId, TaskId};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeliveryTier {
    Interrupt,
    Hook,
    NextTurn,
    NewTurn,
    Digested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedMessage {
    pub message: Message,
    pub tier: DeliveryTier,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Digest {
    pub owner: AgentId,
    pub items: Vec<Message>,
}

impl Digest {
    pub fn render(&self) -> String {
        let mut out = format!(
            "digest for agent {}: {} item(s)\n",
            self.owner.get(),
            self.items.len()
        );
        for message in &self.items {
            out.push_str(&format!(
                "- [{:?}] {}\n",
                message.kind,
                message.body.lines().next().unwrap_or("")
            ));
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InjectionFlag {
    InstructionOverride,
    SystemDisclosure,
    RoleReassignment,
    PrivilegeEscalation,
}

pub fn detect_injection_markers(body: &str) -> Vec<InjectionFlag> {
    let lower = body.to_lowercase();
    let mut flags = Vec::new();
    if [
        "ignore previous instructions",
        "ignore all previous instructions",
        "disregard previous instructions",
        "disregard all prior instructions",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        flags.push(InjectionFlag::InstructionOverride);
    }
    if [
        "repeat your system prompt",
        "reveal your system prompt",
        "print your system instructions",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        flags.push(InjectionFlag::SystemDisclosure);
    }
    if lower.contains("you are now ") {
        flags.push(InjectionFlag::RoleReassignment);
    }
    if ["grant admin", "escalate privileges"]
        .iter()
        .any(|marker| lower.contains(marker))
    {
        flags.push(InjectionFlag::PrivilegeEscalation);
    }
    flags
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteOutcome {
    Queued(Vec<AgentId>),
    Digested(Vec<AgentId>),
    DroppedDuplicate,
    RateLimited(AgentId),
    Quarantined(Vec<InjectionFlag>),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RouteError {
    #[error("unknown sender agent {0}")]
    UnknownSender(AgentId),
    #[error("unknown recipient agent {0}")]
    UnknownRecipient(AgentId),
    #[error("broadcast from non-human sender is forbidden")]
    BroadcastForbidden,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouterConfig {
    pub max_per_kind_per_minute: u32,
    pub window_ms: u64,
    pub dedupe_cap: usize,
    pub digest_cap: usize,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            max_per_kind_per_minute: 10,
            window_ms: 60_000,
            dedupe_cap: 1024,
            digest_cap: 64,
        }
    }
}

fn tier_rank(tier: DeliveryTier) -> u8 {
    match tier {
        DeliveryTier::Interrupt => 0,
        DeliveryTier::Hook => 1,
        DeliveryTier::NextTurn => 2,
        DeliveryTier::NewTurn => 3,
        DeliveryTier::Digested => 4,
    }
}

fn message_fingerprint(message: &Message) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (message.kind as u8).hash(&mut hasher);
    message.refs.task_id.map(|id| id.get()).hash(&mut hasher);
    message
        .refs
        .contract_id
        .map(|id| id.get())
        .hash(&mut hasher);
    message
        .refs
        .decision_id
        .map(|id| id.get())
        .hash(&mut hasher);
    message.body.hash(&mut hasher);
    hasher.finish()
}

fn kind_is_low(kind: MessageKind) -> bool {
    matches!(kind, MessageKind::StatusMilestone)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Router {
    config: RouterConfig,
    inboxes: HashMap<AgentId, VecDeque<QueuedMessage>>,
    digests: HashMap<AgentId, Vec<Message>>,
    seen: VecDeque<(u64, u64)>,
    rates: HashMap<(AgentId, u8), Vec<u64>>,
}

impl Router {
    pub fn new(config: RouterConfig) -> Self {
        Self {
            config,
            inboxes: HashMap::new(),
            digests: HashMap::new(),
            seen: VecDeque::new(),
            rates: HashMap::new(),
        }
    }

    pub fn inbox(&self, agent: AgentId) -> Vec<QueuedMessage> {
        self.inboxes
            .get(&agent)
            .map(|queue| queue.iter().cloned().collect())
            .unwrap_or_default()
    }

    pub fn drain(&mut self, agent: AgentId) -> Vec<QueuedMessage> {
        self.inboxes
            .remove(&agent)
            .map(|queue| queue.into_iter().collect())
            .unwrap_or_default()
    }

    pub fn take_digest(&mut self, agent: AgentId) -> Option<Digest> {
        self.digests.remove(&agent).map(|items| Digest {
            owner: agent,
            items,
        })
    }

    pub fn delivery_for(
        &self,
        graph: &TaskGraph,
        to: AgentId,
        kind: MessageKind,
        refs_task: Option<TaskId>,
        refs_contract: Option<ContractId>,
        _refs_decision: Option<DecisionId>,
    ) -> DeliveryTier {
        if kind_is_low(kind) {
            return DeliveryTier::Digested;
        }
        let state = graph
            .agents
            .get(&to)
            .map(|agent| agent.state)
            .unwrap_or(AgentState::Dead);
        if state == AgentState::Idle {
            return DeliveryTier::NewTurn;
        }
        match kind {
            MessageKind::Blocker | MessageKind::Escalation => DeliveryTier::Interrupt,
            MessageKind::ContractProposal
            | MessageKind::ContractAccept
            | MessageKind::ContractChange => {
                if self.holds_active_lease(graph, to, refs_task, refs_contract) {
                    DeliveryTier::Interrupt
                } else {
                    DeliveryTier::Hook
                }
            }
            _ => DeliveryTier::NextTurn,
        }
    }

    fn holds_active_lease(
        &self,
        graph: &TaskGraph,
        agent: AgentId,
        refs_task: Option<TaskId>,
        refs_contract: Option<ContractId>,
    ) -> bool {
        let mut party_tasks: Vec<TaskId> = Vec::new();
        if let Some(task_id) = refs_task {
            party_tasks.push(task_id);
        }
        if let Some(contract_id) = refs_contract
            && let Some(contract) = graph.contracts.get(&contract_id)
        {
            party_tasks.extend(contract.parties.iter().copied());
        }
        party_tasks.iter().any(|task_id| {
            graph.tasks.get(task_id).is_some_and(|task| {
                task.lease
                    .as_ref()
                    .is_some_and(|lease| lease.owner == agent)
            })
        })
    }

    fn affected_owners(&self, graph: &TaskGraph, message: &Message) -> BTreeSet<AgentId> {
        let mut owners = BTreeSet::new();
        if let Some(task_id) = message.refs.task_id
            && let Some(task) = graph.tasks.get(&task_id)
            && let Some(owner) = task.owner
        {
            owners.insert(owner);
        }
        if let Some(contract_id) = message.refs.contract_id
            && let Some(contract) = graph.contracts.get(&contract_id)
        {
            for party in &contract.parties {
                if let Some(task) = graph.tasks.get(party)
                    && let Some(owner) = task.owner
                {
                    owners.insert(owner);
                }
            }
        }
        owners
    }

    pub fn ingest(
        &mut self,
        graph: &TaskGraph,
        message: &Message,
        at_ms: u64,
    ) -> Result<RouteOutcome, RouteError> {
        match &message.sender {
            Actor::Agent { agent_id } if !graph.agents.contains_key(agent_id) => {
                return Err(RouteError::UnknownSender(*agent_id));
            }
            _ => {}
        }
        for recipient in &message.recipients {
            if !graph.agents.contains_key(recipient) {
                return Err(RouteError::UnknownRecipient(*recipient));
            }
        }
        if message.recipients.len() > 1 && !matches!(message.sender, Actor::Human | Actor::Host) {
            return Err(RouteError::BroadcastForbidden);
        }
        let flags = detect_injection_markers(&message.body);
        if !flags.is_empty() {
            return Ok(RouteOutcome::Quarantined(flags));
        }
        let fingerprint = message_fingerprint(message);
        if self.seen.iter().any(|(hash, _)| *hash == fingerprint) {
            return Ok(RouteOutcome::DroppedDuplicate);
        }
        let sender_key = match &message.sender {
            Actor::Agent { agent_id } => Some(*agent_id),
            _ => None,
        };
        if let Some(sender) = sender_key {
            let rate_key = (sender, message.kind as u8);
            let window_start = at_ms.saturating_sub(self.config.window_ms);
            let recent = self.rates.entry(rate_key).or_default();
            recent.retain(|ts| *ts > window_start);
            if recent.len() as u32 >= self.config.max_per_kind_per_minute {
                return Ok(RouteOutcome::RateLimited(sender));
            }
            recent.push(at_ms);
        }
        self.seen.push_back((fingerprint, at_ms));
        while self.seen.len() > self.config.dedupe_cap {
            self.seen.pop_front();
        }
        let mut recipients: BTreeSet<AgentId> = message.recipients.iter().copied().collect();
        recipients.extend(self.affected_owners(graph, message));
        let mut delivered = Vec::new();
        let mut digested = Vec::new();
        for recipient in recipients {
            let tier = self.delivery_for(
                graph,
                recipient,
                message.kind,
                message.refs.task_id,
                message.refs.contract_id,
                message.refs.decision_id,
            );
            if tier == DeliveryTier::Digested {
                let buffer = self.digests.entry(recipient).or_default();
                if buffer.len() < self.config.digest_cap {
                    buffer.push(message.clone());
                }
                digested.push(recipient);
                continue;
            }
            let queue = self.inboxes.entry(recipient).or_default();
            let position = queue
                .iter()
                .position(|queued| tier_rank(queued.tier) > tier_rank(tier))
                .unwrap_or(queue.len());
            queue.insert(
                position,
                QueuedMessage {
                    message: message.clone(),
                    tier,
                },
            );
            delivered.push(recipient);
        }
        if !digested.is_empty() && delivered.is_empty() {
            return Ok(RouteOutcome::Digested(digested));
        }
        if !digested.is_empty() {
            for recipient in digested {
                delivered.push(recipient);
            }
        }
        Ok(RouteOutcome::Queued(delivered))
    }
}
