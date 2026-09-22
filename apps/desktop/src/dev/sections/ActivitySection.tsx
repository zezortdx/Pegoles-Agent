import { ActivityItem, ActivityList } from "@pegoles/ui";
import { ACTIVITY_FIXTURES } from "../fixtures";
import { LabSection, SimulatedTag } from "../lab/controls";

export function ActivitySection() {
  return (
    <LabSection
      id="lab-activity"
      index="07"
      title="Activity"
      lead="A calm timeline of things that happened — not a heartbeat log. Routine lifecycle recedes, repeats collapse into one row, failures keep their shape."
    >
      <div className="lab-activity">
        <SimulatedTag>Fixture events (real app: Core AgentEvents)</SimulatedTag>
        <ActivityList label="Pegoles Computer activity">
          {ACTIVITY_FIXTURES.map((event) => (
            <ActivityItem
              key={event.id}
              kind={event.kind}
              title={event.title}
              detail={event.detail}
              at={event.at}
              tone={event.tone}
              repeatCount={event.repeatCount}
              emphasis={event.emphasis}
            />
          ))}
        </ActivityList>
      </div>
    </LabSection>
  );
}
