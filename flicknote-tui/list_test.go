package main

import "testing"

func TestTypeIconUsesMicrophoneForMeeting(t *testing.T) {
	t.Parallel()

	if got := typeIcon("meeting"); got != "🎙" {
		t.Fatalf("typeIcon(meeting) = %q, want microphone", got)
	}
}
