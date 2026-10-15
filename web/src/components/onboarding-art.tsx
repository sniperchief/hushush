// Onboarding illustrations (decorative). The amber dotted lines are pay
// streams; they flow unless the visitor prefers reduced motion.

const sans = "var(--font-landing)";
const mono = "var(--font-plex)";
const flow = "animate-flow motion-reduce:animate-none";

/** "I pay a team": one company streaming to three people. */
export function PayTeamArt() {
  const lines = ["M66 70 C130 70 130 30 196 30", "M66 70 H196", "M66 70 C130 70 130 110 196 110"];
  const people = [
    { y: 14, initials: "AN", name: "Ava", rate: "1.5/30d", fill: "#f8da9d" },
    { y: 56, initials: "ML", name: "Marco", rate: "9/30d", fill: "#d5befa" },
    { y: 98, initials: "PR", name: "Priya", rate: "0.2/day", fill: "#e3dfd5" },
  ];
  return (
    <svg viewBox="0 0 340 140" width="100%" height="100%" preserveAspectRatio="xMidYMid meet" aria-hidden="true" className="block">
      <g fill="none" stroke="#e3dfd5" strokeWidth="2">
        {lines.map((d) => (
          <path key={d} d={d} />
        ))}
      </g>
      <g fill="none" stroke="#f9a600" strokeWidth="3" strokeLinecap="round" strokeDasharray="3 11" className={flow}>
        {lines.map((d) => (
          <path key={d} d={d} />
        ))}
      </g>
      <rect x="14" y="44" width="52" height="52" rx="12" fill="#261b07" />
      <text x="40" y="75" textAnchor="middle" fill="#f8f7f5" fontSize="15" fontWeight="600" style={{ fontFamily: sans }}>
        KC
      </text>
      <text x="40" y="116" textAnchor="middle" fill="#61594a" fontSize="10" style={{ fontFamily: sans }}>
        Your company
      </text>
      <g style={{ fontFamily: sans }} fontSize="11">
        {people.map((p) => (
          <g key={p.name}>
            <rect x="196" y={p.y} width="132" height="32" rx="16" fill="#ffffff" stroke="#e3dfd5" />
            <circle cx="213" cy={p.y + 16} r="10" fill={p.fill} />
            <text x="213" y={p.y + 19.5} textAnchor="middle" fill="#261b07" fontSize="8.5" fontWeight="600">
              {p.initials}
            </text>
            <text x="230" y={p.y + 20} fill="#261b07" fontWeight="600">
              {p.name}
            </text>
            <text x="318" y={p.y + 20} textAnchor="end" fill="#61594a" fontSize="9.5" style={{ fontFamily: mono }}>
              {p.rate}
            </text>
          </g>
        ))}
      </g>
    </svg>
  );
}

/** "I get paid": a company streaming into a growing balance. */
export function GetPaidArt() {
  const line = "M44 70 C80 70 84 70 116 70";
  return (
    <svg viewBox="0 0 340 140" width="100%" height="100%" preserveAspectRatio="xMidYMid meet" aria-hidden="true" className="block">
      <path d={line} fill="none" stroke="#e3dfd5" strokeWidth="2" />
      <path d={line} fill="none" stroke="#f9a600" strokeWidth="3" strokeLinecap="round" strokeDasharray="3 11" className={flow} />
      <circle cx="26" cy="70" r="18" fill="#261b07" />
      <text x="26" y="74.5" textAnchor="middle" fill="#f8f7f5" fontSize="11" fontWeight="600" style={{ fontFamily: sans }}>
        KC
      </text>
      <rect x="116" y="18" width="212" height="104" rx="12" fill="#ffffff" stroke="#e3dfd5" />
      <text x="134" y="42" fill="#61594a" fontSize="10" style={{ fontFamily: sans }}>
        Earned so far
      </text>
      <text x="134" y="72" fill="#261b07" fontSize="21" fontWeight="500" style={{ fontFamily: mono }}>
        1.24800000
      </text>
      <text x="276" y="72" fill="#61594a" fontSize="11" style={{ fontFamily: sans }}>
        ZEC
      </text>
      <circle cx="138" cy="99" r="3.5" fill="#f9a600" />
      <text x="147" y="102.5" fill="#61594a" fontSize="9.5" style={{ fontFamily: sans }}>
        Growing every second
      </text>
      <rect x="254" y="88" width="62" height="24" rx="6" fill="#f9a600" />
      <text x="285" y="104" textAnchor="middle" fill="#261b07" fontSize="10" fontWeight="600" style={{ fontFamily: sans }}>
        Withdraw
      </text>
    </svg>
  );
}
