import {
  faEnvelope,
  faBriefcase,
  faHouse,
  faStar,
  faGraduationCap,
  faFlask,
  faHeart,
  faInbox,
} from "@fortawesome/free-solid-svg-icons";
/** @type {Array<[typeof faEnvelope,string]>} */
const choices = [
  [faEnvelope, "Конверт"],
  [faBriefcase, "Работа"],
  [faHouse, "Дом"],
  [faStar, "Звезда"],
  [faGraduationCap, "Учёба"],
  [faFlask, "Наука"],
  [faHeart, "Личное"],
  [faInbox, "Лоток"],
];
export const iconOptions = choices.map(([icon, label]) => [
  icon.iconName,
  label,
]);
export function accountIcon(name) {
  const [width, height, , , path] = (choices.find(
    ([icon]) => icon.iconName === name,
  ) || choices[0])[0].icon;
  return `<svg width="16" height="16" aria-hidden="true" viewBox="0 0 ${width} ${height}"><path fill="currentColor" d="${path}"></path></svg>`;
}
