package codex

import "time"

// TransferResult is the broker's authoritative terminal outcome. A timeout at
// either window does not establish whether the other window committed.
type TransferResult struct {
	State string `json:"state"`
}

const (
	TransferCancelled    = "cancelled"
	TransferCommitted    = "committed"
	transferReceiptLimit = 256
)

type transferReceipt struct {
	owner, claimed string
	state          string
	expires        time.Time
}

// Caller holds b.mu. Receipts contain no document data or peer references.
func (b *Broker) recordTransfer(ticket string, t transfer, state string) {
	now := time.Now()
	for key, receipt := range b.transferReceipts {
		if !now.Before(receipt.expires) {
			delete(b.transferReceipts, key)
		}
	}
	if b.transferReceipts == nil {
		b.transferReceipts = make(map[string]transferReceipt)
	}
	if len(b.transferReceipts) >= transferReceiptLimit {
		var oldest string
		var expiry time.Time
		for key, receipt := range b.transferReceipts {
			if oldest == "" || receipt.expires.Before(expiry) {
				oldest, expiry = key, receipt.expires
			}
		}
		delete(b.transferReceipts, oldest)
	}
	receipt := transferReceipt{state: state, expires: now.Add(2 * time.Minute)}
	if t.Owner != nil {
		receipt.owner = t.Owner.id
	}
	if t.Claimed != nil {
		receipt.claimed = t.Claimed.id
	}
	b.transferReceipts[ticket] = receipt
}

// Caller holds b.mu. Outcomes are visible only to the original participants.
func (b *Broker) transferOutcome(ticket string, p *peer) string {
	r, ok := b.transferReceipts[ticket]
	if !ok || !time.Now().Before(r.expires) {
		delete(b.transferReceipts, ticket)
		return ""
	}
	if p.id != r.owner && p.id != r.claimed {
		return ""
	}
	return r.state
}
