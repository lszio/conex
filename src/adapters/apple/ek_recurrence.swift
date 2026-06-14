import Foundation
import EventKit

// MARK: - JSON Encoding

struct RecurrenceJSON: Codable {
    var frequency: String       // "daily" | "weekly" | "monthly" | "yearly"
    var interval: Int
    var days_of_week: [Int]?
    var days_of_month: [Int]?
    var months_of_year: [Int]?
    var weeks_of_the_year: [Int]?
    var days_of_the_year: [Int]?
    var set_positions: [Int]?
    var end_date: String?       // ISO 8601
    var occurrence_count: Int?
}

// MARK: - EventKit Helpers

func getEKStore() -> EKEventStore {
    return EKEventStore()
}

func findReminder(by id: String, in store: EKEventStore) -> EKReminder? {
    let ekID: String
    if id.hasPrefix("x-apple-reminder://") {
        ekID = String(id.dropFirst("x-apple-reminder://".count))
    } else {
        ekID = id
    }
    
    let calendars = store.calendars(for: .reminder)
    let predicate = store.predicateForReminders(in: calendars)
    var found: EKReminder? = nil
    let group = DispatchGroup()
    group.enter()
    store.fetchReminders(matching: predicate) { reminders in
        if let rs = reminders {
            // Try exact match first, then suffix match
            if let exact = rs.first(where: { $0.calendarItemIdentifier == ekID }) {
                found = exact
            } else {
                found = rs.first(where: { $0.calendarItemIdentifier.hasSuffix(ekID) || ekID.hasSuffix($0.calendarItemIdentifier) })
            }
        }
        group.leave()
    }
    group.wait()
    return found
}

// MARK: - Commands

func cmdRead(reminderID: String) {
    let store = getEKStore()
    guard let reminder = findReminder(by: reminderID, in: store) else {
        print("ERROR:reminder_not_found")
        exit(1)
    }
    guard let rules = reminder.recurrenceRules, !rules.isEmpty else {
        print("{}")
        return
    }
    
    let rule = rules[0]
    var json = RecurrenceJSON(frequency: "daily", interval: rule.interval)
    
    switch rule.frequency {
    case .daily: json.frequency = "daily"
    case .weekly: json.frequency = "weekly"
    case .monthly: json.frequency = "monthly"
    case .yearly: json.frequency = "yearly"
    @unknown default: json.frequency = "daily"
    }
    
    if let days = rule.daysOfTheWeek {
        json.days_of_week = days.map { $0.dayOfTheWeek.rawValue }
    }
    if let dom = rule.daysOfTheMonth {
        json.days_of_month = dom.map { $0.intValue }
    }
    if let moy = rule.monthsOfTheYear {
        json.months_of_year = moy.map { $0.intValue }
    }
    if let woy = rule.weeksOfTheYear {
        json.weeks_of_the_year = woy.map { $0.intValue }
    }
    if let doy = rule.daysOfTheYear {
        json.days_of_the_year = doy.map { $0.intValue }
    }
    if let sp = rule.setPositions {
        json.set_positions = sp.map { $0.intValue }
    }
    if let end = rule.recurrenceEnd {
        if let ed = end.endDate {
            let formatter = ISO8601DateFormatter()
            json.end_date = formatter.string(from: ed)
        }
        if end.occurrenceCount > 0 {
            json.occurrence_count = end.occurrenceCount
        }
    }
    
    let encoder = JSONEncoder()
    encoder.outputFormatting = .init(rawValue: 0) // compact
    if let data = try? encoder.encode(json), let str = String(data: data, encoding: .utf8) {
        print(str)
    } else {
        print("{}")
    }
}

func countDayOfWeek(_ name: String) -> EKWeekday? {
    let map: [String: EKWeekday] = [
        "sun": .sunday, "mon": .monday, "tue": .tuesday, "wed": .wednesday,
        "thu": .thursday, "fri": .friday, "sat": .saturday,
        "1": .sunday, "2": .monday, "3": .tuesday, "4": .wednesday,
        "5": .thursday, "6": .friday, "7": .saturday,
    ]
    return map[name.lowercased()]
}

func cmdWrite(reminderID: String, jsonStr: String) {
    guard let data = jsonStr.data(using: .utf8),
          let json = try? JSONDecoder().decode(RecurrenceJSON.self, from: data) else {
        print("ERROR:invalid_json")
        exit(1)
    }
    
    let store = getEKStore()
    guard let reminder = findReminder(by: reminderID, in: store) else {
        print("ERROR:reminder_not_found")
        exit(1)
    }
    
    let frequency: EKRecurrenceFrequency
    switch json.frequency {
    case "daily": frequency = .daily
    case "weekly": frequency = .weekly
    case "monthly": frequency = .monthly
    case "yearly": frequency = .yearly
    default:
        print("ERROR:invalid_frequency")
        exit(1)
    }
    
    var daysOfWeek: [EKRecurrenceDayOfWeek]? = nil
    if let dow = json.days_of_week {
        daysOfWeek = dow.compactMap { v in
            guard let wd = EKWeekday(rawValue: v) else { return nil }
            return EKRecurrenceDayOfWeek(wd)
        }
        if daysOfWeek?.isEmpty == true { daysOfWeek = nil }
    }
    
    var daysOfMonth: [NSNumber]? = json.days_of_month?.map { NSNumber(value: $0) }
    if daysOfMonth?.isEmpty == true { daysOfMonth = nil }
    
    var monthsOfYear: [NSNumber]? = json.months_of_year?.map { NSNumber(value: $0) }
    if monthsOfYear?.isEmpty == true { monthsOfYear = nil }
    
    var weeksOfYear: [NSNumber]? = json.weeks_of_the_year?.map { NSNumber(value: $0) }
    if weeksOfYear?.isEmpty == true { weeksOfYear = nil }
    
    var daysOfYear: [NSNumber]? = json.days_of_the_year?.map { NSNumber(value: $0) }
    if daysOfYear?.isEmpty == true { daysOfYear = nil }
    
    var setPositions: [NSNumber]? = json.set_positions?.map { NSNumber(value: $0) }
    if setPositions?.isEmpty == true { setPositions = nil }
    
    var recurrenceEnd: EKRecurrenceEnd? = nil
    if let endDateStr = json.end_date {
        let formatter = ISO8601DateFormatter()
        if let date = formatter.date(from: endDateStr) {
            recurrenceEnd = EKRecurrenceEnd(end: date)
        }
    } else if let count = json.occurrence_count, count > 0 {
        recurrenceEnd = EKRecurrenceEnd(occurrenceCount: count)
    }
    
    let rule = EKRecurrenceRule(
        recurrenceWith: frequency,
        interval: json.interval,
        daysOfTheWeek: daysOfWeek,
        daysOfTheMonth: daysOfMonth,
        monthsOfTheYear: monthsOfYear,
        weeksOfTheYear: weeksOfYear,
        daysOfTheYear: daysOfYear,
        setPositions: setPositions,
        end: recurrenceEnd
    )
    
    reminder.recurrenceRules = [rule]
    
    do {
        try store.save(reminder, commit: true)
        print("OK")
    } catch {
        print("ERROR:\(error.localizedDescription)")
        exit(1)
    }
}

func cmdRemove(reminderID: String) {
    let store = getEKStore()
    guard let reminder = findReminder(by: reminderID, in: store) else {
        print("ERROR:reminder_not_found")
        exit(1)
    }
    
    reminder.recurrenceRules = []
    
    do {
        try store.save(reminder, commit: true)
        print("OK")
    } catch {
        print("ERROR:\(error.localizedDescription)")
        exit(1)
    }
}

// MARK: - Main

// macOS 14+: requestFullAccessToReminders is async
func runAsync() async {
    let store = getEKStore()
    do {
        let granted = try await store.requestFullAccessToReminders()
        guard granted else {
            print("ERROR:access_denied")
            exit(1)
        }
    } catch {
        print("ERROR:\(error.localizedDescription)")
        exit(1)
    }
    
    let args = CommandLine.arguments
    guard args.count >= 3 else {
        print("USAGE: ek_recurrence <read|write|remove> <reminder_id> [json]")
        exit(1)
    }
    
    let command = args[1]
    let reminderID = args[2]
    
    switch command {
    case "read":
        cmdRead(reminderID: reminderID)
    case "write":
        guard args.count >= 4 else {
            print("ERROR:missing_json")
            exit(1)
        }
        cmdWrite(reminderID: reminderID, jsonStr: args[3])
    case "remove":
        cmdRemove(reminderID: reminderID)
    default:
        print("ERROR:unknown_command")
        exit(1)
    }
}

if #available(macOS 14.0, *) {
    await runAsync()
} else {
    print("ERROR:requires_macos_14+")
    exit(1)
}
